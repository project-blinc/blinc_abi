//! The cascade: which rules of which sheets an element matches, and the
//! declarations that win, property by property.
//!
//! `!important` stands over normal, then specificity, then order: a later
//! sheet's rule after an earlier sheet's, a later rule after an earlier one,
//! a later declaration after an earlier one. An element's own declarations
//! (its `style`) stand over every rule but an `!important` one. A shorthand
//! sets its longhands anew, over what weaker declarations gave them. Custom
//! properties and the inherited properties pass from parent to child, and
//! `var(--name, fallback)` reads the element's custom properties, then the
//! sheets' `:root` variables, then the theme's, then its fallback.
//!
//! The host owns the tree. It describes each element through [`Tree`] and
//! styles parents before children, handing each child its parent's
//! [`Computed`]. Names are atoms of the cascade's own table, so matching
//! compares integers; each sheet keeps its atoms and a map into that table.

use super::media::MediaEnvironment;
use super::{
    Atom, Atoms, AttributeOp, Combinator, Nth, Pseudo, Rule, STATES, Selector, Stylesheet,
};
use std::collections::HashMap;

/// The properties a child inherits from its parent, besides custom properties.
pub const INHERITED: &[&str] = &[
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "line-height",
    "letter-spacing",
    "text-align",
    "white-space",
];

/// Shorthands and the longhands each sets.
pub fn longhands(name: &str) -> &'static [&'static str] {
    const SIDES_MARGIN: &[&str] = &["margin-top", "margin-right", "margin-bottom", "margin-left"];
    const SIDES_PADDING: &[&str] = &[
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ];
    const BORDER_WIDTH: &[&str] = &[
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
    ];
    const BORDER_COLOR: &[&str] = &[
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
    ];
    const BORDER: &[&str] = &[
        "border-width",
        "border-color",
        "border-style",
        "border-top",
        "border-top-width",
        "border-top-color",
        "border-right",
        "border-right-width",
        "border-right-color",
        "border-bottom",
        "border-bottom-width",
        "border-bottom-color",
        "border-left",
        "border-left-width",
        "border-left-color",
    ];
    match name {
        "margin" => SIDES_MARGIN,
        "padding" => SIDES_PADDING,
        "inset" => &["top", "right", "bottom", "left"],
        "gap" => &["row-gap", "column-gap"],
        "flex" => &["flex-grow", "flex-shrink", "flex-basis"],
        "background" => &["background-color", "background-image"],
        "overflow" => &["overflow-x", "overflow-y"],
        "outline" => &["outline-width", "outline-color"],
        "border-width" => BORDER_WIDTH,
        "border-color" => BORDER_COLOR,
        "border-top" => &["border-top-width", "border-top-color"],
        "border-right" => &["border-right-width", "border-right-color"],
        "border-bottom" => &["border-bottom-width", "border-bottom-color"],
        "border-left" => &["border-left-width", "border-left-color"],
        "border" => BORDER,
        _ => &[],
    }
}

/// An element's state pseudo-classes, a bit per entry of [`STATES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct States(pub u32);

impl States {
    /// The bit of state `name`, one of [`STATES`].
    pub fn bit(name: &str) -> Option<u32> {
        STATES.iter().position(|s| *s == name).map(|i| 1 << i)
    }

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

/// What the cascade needs to know of one element, in the cascade's atoms.
#[derive(Clone, Debug, Default)]
pub struct Element {
    /// Its element types, as a type selector names them; the first is its own, for `:nth-of-type`.
    pub types: Vec<Atom>,
    pub id: Option<Atom>,
    pub classes: Vec<Atom>,
    /// Its attributes, by name.
    pub attributes: Vec<(Atom, Atom)>,
    /// Its own declarations, as `style` gives them, by property name.
    pub inline: Vec<(Atom, String)>,
    pub states: States,
    /// Made by a layout rather than the author, as a table's wrapper: not
    /// counted among its siblings by structural pseudo-classes.
    pub anonymous: bool,
}

/// The host's tree, as the cascade walks it.
pub trait Tree {
    type Node: Copy + Eq + std::hash::Hash;
    fn parent(&self, node: Self::Node) -> Option<Self::Node>;
    /// Every child, in order: elements and other nodes alike.
    fn children(&self, node: Self::Node) -> Vec<Self::Node>;
    /// The node as an element, or none for a node that is not one.
    fn element(&self, node: Self::Node) -> Option<&Element>;
    /// The node `:root` names; none for the one with no parent.
    fn root(&self) -> Option<Self::Node> {
        None
    }
}

/// An element's style: every value it holds, inherited ones and custom
/// properties included, and the declarations it applies, `var()`s resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Computed {
    /// By property name; what a child inherits from.
    pub values: Vec<(Atom, String)>,
    /// What applies to the element: its own declarations, and on a text
    /// element the inherited ones, `var()`s replaced, by property name.
    pub resolved: Vec<(Atom, String)>,
    /// `font-size` in pixels, computed, as its children read `em` from.
    pub font_size: f64,
}

impl Computed {
    pub fn value(&self, name: Atom) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What a match read beyond the element's own names, so the host knows
/// when to style it again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dependencies<N> {
    /// States of nodes, the element's own or others', that a selector tested.
    pub states: Vec<(N, u32)>,
    /// Whether a `var()` was answered by the theme.
    pub theme: bool,
}

#[derive(Clone, Copy)]
struct Entry {
    rule: u32,
    selector: u32,
}

#[derive(Default)]
struct Index {
    ids: HashMap<Atom, Vec<Entry>>,
    classes: HashMap<Atom, Vec<Entry>>,
    types: HashMap<Atom, Vec<Entry>>,
    rest: Vec<Entry>,
}

struct Sheet {
    sheet: Stylesheet,
    /// Each of the sheet's atoms, as the cascade's.
    map: Vec<Atom>,
    index: Index,
    /// Names a selector tests on an element other than the one it styles:
    /// those of every compound but a selector's subject, and all of a
    /// `:has()` argument's. A change to any other name restyles its element alone.
    reach: std::collections::HashSet<Atom>,
    /// Whether any selector uses `:has()`, which a change below or beside an element can answer.
    has: bool,
}

/// A sheet in the cascade, for removing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetId(u32);

/// Sheets in order, an atom table the host's names and theirs share, and
/// what media queries and the theme answer.
pub struct Cascade {
    atoms: Atoms,
    sheets: Vec<(SheetId, Sheet)>,
    next: u32,
    theme: HashMap<Atom, String>,
    env: MediaEnvironment,
    text: Atom,
    root_font_size: f64,
    inherited: Vec<Atom>,
}

impl Default for Cascade {
    fn default() -> Self {
        Self::new()
    }
}

impl Cascade {
    pub fn new() -> Self {
        let mut atoms = Atoms::default();
        let text = atoms.intern("text");
        let inherited = INHERITED.iter().map(|n| atoms.intern(n)).collect();
        Cascade {
            atoms,
            sheets: Vec::new(),
            next: 0,
            theme: HashMap::new(),
            env: MediaEnvironment {
                width: 0.0,
                height: 0.0,
                dark: false,
            },
            text,
            root_font_size: 16.0,
            inherited,
        }
    }

    /// The atom of `name` in the cascade's table, which element names use.
    pub fn intern(&mut self, name: &str) -> Atom {
        self.atoms.intern(name)
    }

    pub fn atoms(&self) -> &Atoms {
        &self.atoms
    }

    pub fn str(&self, a: Atom) -> &str {
        self.atoms.get(a)
    }

    /// The viewport and scheme media queries are asked about.
    pub fn set_environment(&mut self, env: MediaEnvironment) {
        self.env = env;
    }

    /// The theme's custom properties, by name without the `--`, read after the sheets' own.
    pub fn set_theme(&mut self, vars: &[(&str, &str)]) {
        self.theme = vars
            .iter()
            .map(|(k, v)| (self.atoms.intern(k), v.to_string()))
            .collect();
    }

    /// What media queries are asked about, and the root font size.
    pub fn environment(&self) -> (MediaEnvironment, f64) {
        (self.env, self.root_font_size)
    }

    pub fn set_root_font_size(&mut self, px: f64) {
        self.root_font_size = px;
    }

    /// Adds `sheet` after every sheet already in, so its rules win ties of specificity over theirs.
    pub fn push(&mut self, sheet: Stylesheet) -> SheetId {
        let at = self.sheets.len();
        self.insert(at, sheet)
    }

    /// Adds `sheet` at position `at` among the sheets, `0` first.
    pub fn insert(&mut self, at: usize, sheet: Stylesheet) -> SheetId {
        let map: Vec<Atom> = (0..sheet.atoms.len() as u32)
            .map(|i| self.atoms.intern(sheet.str(Atom(i))))
            .collect();
        let mut index = Index::default();
        for (r, rule) in sheet.rules.iter().enumerate() {
            for s in rule.selectors.range() {
                let entry = Entry {
                    rule: r as u32,
                    selector: s as u32,
                };
                let subject = sheet.subject(&sheet.selectors[s]);
                // Keyed by what the subject must have, most selective first: an element is only tried against a key it holds.
                if let Some(id) = subject.id {
                    index.ids.entry(map[id.0 as usize]).or_default().push(entry);
                } else if let Some(&class) = sheet.compound_classes(subject).first() {
                    index
                        .classes
                        .entry(map[class.0 as usize])
                        .or_default()
                        .push(entry);
                } else if let Some(t) = subject.type_name {
                    index
                        .types
                        .entry(map[t.0 as usize])
                        .or_default()
                        .push(entry);
                } else {
                    index.rest.push(entry);
                }
            }
        }
        // Which names reach past the element they are on: see `Sheet::reach`.
        let mut reach = std::collections::HashSet::new();
        let mut has = false;
        let names = |c: &super::Compound, reach: &mut std::collections::HashSet<Atom>| {
            let m = |a: Atom| map[a.0 as usize];
            reach.extend(c.type_name.map(m));
            reach.extend(c.id.map(m));
            reach.extend(sheet.compound_classes(c).iter().map(|&a| m(a)));
            reach.extend(sheet.compound_attributes(c).iter().map(|a| m(a.name)));
        };
        for s in &sheet.selectors {
            let compounds = sheet.selector_compounds(s);
            for c in &compounds[..compounds.len() - 1] {
                names(c, &mut reach);
            }
            for c in compounds {
                for p in sheet.compound_pseudos(c) {
                    if let Pseudo::Has(list) = *p {
                        has = true;
                        for inner in sheet.selector_list(list) {
                            for c in sheet.selector_compounds(inner) {
                                names(c, &mut reach);
                            }
                        }
                    }
                }
            }
        }
        let id = SheetId(self.next);
        self.next += 1;
        self.sheets.insert(
            at.min(self.sheets.len()),
            (
                id,
                Sheet {
                    sheet,
                    map,
                    index,
                    reach,
                    has,
                },
            ),
        );
        id
    }

    /// Whether a selector tests `name` on an element other than the one it styles.
    pub fn reaches(&self, name: Atom) -> bool {
        self.sheets.iter().any(|(_, s)| s.reach.contains(&name))
    }

    /// Whether a child takes `name` from its parent: an inherited property or a custom property.
    pub fn inherits(&self, name: Atom) -> bool {
        self.inherited.contains(&name) || self.str(name).starts_with("--")
    }

    /// Whether any sheet uses `:has()`.
    pub fn uses_has(&self) -> bool {
        self.sheets.iter().any(|(_, s)| s.has)
    }

    /// Takes the sheet out; false when it was not in.
    pub fn remove(&mut self, id: SheetId) -> bool {
        let before = self.sheets.len();
        self.sheets.retain(|(s, _)| *s != id);
        self.sheets.len() != before
    }

    /// `node`'s style, from its parent's (none at the top), and what its match depended on.
    pub fn style<T: Tree>(
        &self,
        tree: &T,
        node: T::Node,
        parent: Option<&Computed>,
    ) -> (Computed, Dependencies<T::Node>) {
        let mut deps = Dependencies {
            states: Vec::new(),
            theme: false,
        };
        let Some(element) = tree.element(node) else {
            return (
                Computed {
                    font_size: parent.map_or(self.root_font_size, |p| p.font_size),
                    ..Default::default()
                },
                deps,
            );
        };
        let walk = Walk {
            tree,
            cascade: self,
        };

        // The declarations of every rule that matches, in cascade order.
        struct Matched {
            important: bool,
            specificity: u32,
            sheet: usize,
            order: u32,
            index: usize,
            name: Atom,
            value: Atom,
        }
        let mut matched = Vec::new();
        for (s, (_, sheet)) in self.sheets.iter().enumerate() {
            let mut entries: Vec<Entry> = sheet.index.rest.clone();
            let mut add = |list: Option<&Vec<Entry>>| {
                if let Some(l) = list {
                    entries.extend_from_slice(l);
                }
            };
            if let Some(id) = element.id {
                add(sheet.index.ids.get(&id));
            }
            for c in &element.classes {
                add(sheet.index.classes.get(c));
            }
            for t in &element.types {
                add(sheet.index.types.get(t));
            }
            // A rule whose selector list keys it under two of the element's names is matched once per selector, as each is.
            entries.sort_by_key(|e| (e.rule, e.selector));
            entries.dedup_by_key(|e| (e.rule, e.selector));
            for e in entries {
                let rule: &Rule = &sheet.sheet.rules[e.rule as usize];
                if !sheet.sheet.media_holds(rule, &self.env) {
                    continue;
                }
                let selector = &sheet.sheet.selectors[e.selector as usize];
                if !walk.matches(sheet, selector, node, &mut deps) {
                    continue;
                }
                for (i, d) in sheet.sheet.rule_declarations(rule).iter().enumerate() {
                    matched.push(Matched {
                        important: d.important,
                        specificity: selector.specificity,
                        sheet: s,
                        order: rule.order,
                        index: i,
                        name: sheet.map[d.name.0 as usize],
                        value: sheet.map[d.value.0 as usize],
                    });
                }
            }
        }
        matched.sort_by_key(|m| (m.important, m.specificity, m.sheet, m.order, m.index));

        // The element's own declarations stand over every rule but an !important one, which the sort put last.
        let mut own: Vec<(Atom, &str)> = Vec::new();
        fn set<'a>(cascade: &Cascade, own: &mut Vec<(Atom, &'a str)>, name: Atom, value: &'a str) {
            let covered = longhands(cascade.str(name));
            if !covered.is_empty() {
                own.retain(|(k, _)| !covered.contains(&cascade.str(*k)));
            }
            match own.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 = value,
                None => own.push((name, value)),
            }
        }
        let mut inlined = false;
        for m in &matched {
            if m.important && !inlined {
                inlined = true;
                for (k, v) in &element.inline {
                    set(self, &mut own, *k, v.as_str());
                }
            }
            set(self, &mut own, m.name, self.str(m.value));
        }
        if !inlined {
            for (k, v) in &element.inline {
                set(self, &mut own, *k, v.as_str());
            }
        }

        // What it inherits from its parent, under its own.
        let mut values: Vec<(Atom, String)> = parent
            .map(|p| {
                p.values
                    .iter()
                    .filter(|(k, _)| self.inherited.contains(k) || self.str(*k).starts_with("--"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for &(k, v) in &own {
            let v = v.to_string();
            match values.iter_mut().find(|(n, _)| *n == k) {
                Some(slot) => slot.1 = v,
                None => values.push((k, v)),
            }
        }

        // Inherited font properties reach a text element alone; other elements apply only their own.
        let text = element.types.contains(&self.text);
        let mut resolved = Vec::new();
        for (k, v) in &values {
            let name = self.str(*k);
            if name.starts_with("--") || (!text && !own.iter().any(|(o, _)| o == k)) {
                continue;
            }
            resolved.push((*k, self.substitute(v, &values, &mut deps.theme)));
        }

        // `font-size` passes down computed, in pixels.
        let inherited_size = parent.map_or(self.root_font_size, |p| p.font_size);
        let mut font_size = inherited_size;
        if let Some((_, v)) = resolved.iter().find(|(k, _)| self.str(*k) == "font-size") {
            font_size = self.pixels_or(v, inherited_size);
            let px = format!("{}px", super::json::number(font_size));
            if let Some(slot) = values.iter_mut().find(|(k, _)| self.str(*k) == "font-size") {
                slot.1 = px;
            }
        }
        (
            Computed {
                values,
                resolved,
                font_size,
            },
            deps,
        )
    }

    /// `value` with each `var(--name, fallback)` replaced, ten passes deep at most, so a variable naming itself ends.
    fn substitute(&self, value: &str, values: &[(Atom, String)], theme_read: &mut bool) -> String {
        let mut value = value.to_string();
        let mut depth = 0;
        while value.contains("var(") && depth < 10 {
            depth += 1;
            let mut out = String::new();
            let mut from = 0;
            while let Some(rel) = value[from..].find("var(") {
                let at = from + rel;
                let (mut nesting, mut end) = (0, at + 4);
                let bytes = value.as_bytes();
                while end < bytes.len() {
                    match bytes[end] {
                        b'(' => nesting += 1,
                        b')' if nesting == 0 => break,
                        b')' => nesting -= 1,
                        _ => {}
                    }
                    end += 1;
                }
                let args = &value[at + 4..end.min(value.len())];
                let (name, fallback) = match args.find(',') {
                    Some(c) => (args[..c].trim(), Some(args[c + 1..].trim())),
                    None => (args.trim(), None),
                };
                let bare = name.strip_prefix("--").unwrap_or(name);
                let found = self
                    .atoms
                    .find(name)
                    .and_then(|a| values.iter().find(|(k, _)| *k == a).map(|(_, v)| v.clone()))
                    .or_else(|| {
                        // A later sheet's :root variable over an earlier's.
                        self.sheets
                            .iter()
                            .rev()
                            .find_map(|(_, s)| s.sheet.variable(bare).map(str::to_string))
                    })
                    .or_else(|| {
                        let v = self
                            .atoms
                            .find(bare)
                            .and_then(|a| self.theme.get(&a).cloned());
                        if v.is_some() {
                            *theme_read = true;
                        }
                        v
                    })
                    .unwrap_or_else(|| fallback.unwrap_or("").to_string());
                out.push_str(&value[from..at]);
                out.push_str(&found);
                from = (end + 1).min(value.len());
            }
            out.push_str(&value[from..]);
            value = out;
        }
        value
    }

    /// A length in pixels: `px`, `em` and `%` of `of`, `rem`, the viewport's units; `of` when it is none of these.
    fn pixels_or(&self, v: &str, of: f64) -> f64 {
        match super::value::dimension(&v.trim().to_lowercase()) {
            Some((x, u)) => match u.as_str() {
                "px" => x,
                "em" => x * of,
                "%" => x / 100.0 * of,
                "rem" => x * self.root_font_size,
                "vw" => x / 100.0 * self.env.width,
                "vh" => x / 100.0 * self.env.height,
                "vmin" => x / 100.0 * self.env.width.min(self.env.height),
                "vmax" => x / 100.0 * self.env.width.max(self.env.height),
                "" if x == 0.0 => 0.0,
                _ => of,
            },
            None => of,
        }
    }
}

/// Matching selectors against the host's tree.
struct Walk<'t, T: Tree> {
    tree: &'t T,
    cascade: &'t Cascade,
}

impl<T: Tree> Walk<'_, T> {
    fn parent(&self, n: T::Node) -> Option<T::Node> {
        self.tree.parent(n)
    }

    fn ancestors(&self, n: T::Node) -> Vec<T::Node> {
        let mut out = Vec::new();
        let mut at = self.parent(n);
        while let Some(p) = at {
            out.push(p);
            at = self.parent(p);
        }
        out
    }

    /// `n` and its siblings, those a layout added aside left out.
    fn siblings(&self, n: T::Node) -> Vec<T::Node> {
        match self.parent(n) {
            None => vec![n],
            Some(p) => self
                .tree
                .children(p)
                .into_iter()
                .filter(|&c| c == n || self.tree.element(c).is_none_or(|e| !e.anonymous))
                .collect(),
        }
    }

    fn matches(
        &self,
        sheet: &Sheet,
        s: &Selector,
        n: T::Node,
        deps: &mut Dependencies<T::Node>,
    ) -> bool {
        self.at(sheet, s, s.compounds.len as usize - 1, n, None, deps)
    }

    /// Whether compound `i` of `s` matches `n`, and those before it match
    /// where its combinators say, right to left; in a `:has()` argument the
    /// first compound must also stand to `anchor` as its leading combinator says.
    fn at(
        &self,
        sheet: &Sheet,
        s: &Selector,
        i: usize,
        n: T::Node,
        anchor: Option<T::Node>,
        deps: &mut Dependencies<T::Node>,
    ) -> bool {
        let compounds = sheet.sheet.selector_compounds(s);
        if !self.compound(sheet, &compounds[i], n, deps) {
            return false;
        }
        if i == 0 {
            return anchor
                .is_none_or(|a| self.related(a, n, s.leading.unwrap_or(Combinator::Descendant)));
        }
        match sheet.sheet.selector_combinators(s)[i - 1] {
            Combinator::Child => self
                .parent(n)
                .is_some_and(|p| self.at(sheet, s, i - 1, p, anchor, deps)),
            Combinator::Descendant => self
                .ancestors(n)
                .into_iter()
                .any(|a| self.at(sheet, s, i - 1, a, anchor, deps)),
            Combinator::NextSibling => {
                let sib = self.siblings(n);
                let k = sib.iter().position(|&x| x == n).unwrap_or(0);
                k > 0 && self.at(sheet, s, i - 1, sib[k - 1], anchor, deps)
            }
            Combinator::LaterSibling => {
                let sib = self.siblings(n);
                let k = sib.iter().position(|&x| x == n).unwrap_or(0);
                sib[..k]
                    .iter()
                    .any(|&x| self.at(sheet, s, i - 1, x, anchor, deps))
            }
        }
    }

    /// Whether `n` stands to `anchor` as `how` says: inside it, its child, the next sibling or a later one.
    fn related(&self, anchor: T::Node, n: T::Node, how: Combinator) -> bool {
        match how {
            Combinator::Descendant => self.ancestors(n).contains(&anchor),
            Combinator::Child => self.parent(n) == Some(anchor),
            Combinator::NextSibling => {
                let sib = self.siblings(n);
                let k = sib.iter().position(|&x| x == n).unwrap_or(0);
                k > 0 && sib[k - 1] == anchor
            }
            Combinator::LaterSibling => {
                let sib = self.siblings(n);
                let k = sib.iter().position(|&x| x == n);
                let j = sib.iter().position(|&x| x == anchor);
                matches!((j, k), (Some(j), Some(k)) if j < k)
            }
        }
    }

    fn compound(
        &self,
        sheet: &Sheet,
        c: &super::Compound,
        n: T::Node,
        deps: &mut Dependencies<T::Node>,
    ) -> bool {
        let Some(e) = self.tree.element(n) else {
            return false;
        };
        let m = |a: Atom| sheet.map[a.0 as usize];
        if c.type_name.is_some_and(|t| !e.types.contains(&m(t))) {
            return false;
        }
        if c.id.is_some_and(|id| e.id != Some(m(id))) {
            return false;
        }
        if c.pseudo_element.is_some() {
            return false;
        }
        for &class in sheet.sheet.compound_classes(c) {
            if !e.classes.contains(&m(class)) {
                return false;
            }
        }
        for a in sheet.sheet.compound_attributes(c) {
            let have = e
                .attributes
                .iter()
                .find(|(k, _)| *k == m(a.name))
                .map(|(_, v)| self.cascade.str(*v));
            if !attribute(have, a.test.map(|(op, v)| (op, sheet.sheet.str(v)))) {
                return false;
            }
        }
        for p in sheet.sheet.compound_pseudos(c) {
            if !self.pseudo(sheet, p, n, e, deps) {
                return false;
            }
        }
        true
    }

    fn pseudo(
        &self,
        sheet: &Sheet,
        p: &Pseudo,
        n: T::Node,
        e: &Element,
        deps: &mut Dependencies<T::Node>,
    ) -> bool {
        let index =
            |list: &[T::Node]| list.iter().position(|&x| x == n).map_or(0, |i| i + 1) as i32;
        let of_type = |list: Vec<T::Node>| -> Vec<T::Node> {
            let own = e.types.first().copied();
            list.into_iter()
                .filter(|&x| {
                    self.tree
                        .element(x)
                        .is_some_and(|o| o.types.first().copied() == own)
                })
                .collect()
        };
        match *p {
            Pseudo::State(name) => {
                let name = sheet.sheet.str(name);
                let enabled = name == "enabled";
                let bit = States::bit(if enabled { "disabled" } else { name }).unwrap_or(0);
                deps.states.push((n, bit));
                e.states.has(bit) != enabled
            }
            Pseudo::Root => match self.tree.root() {
                Some(r) => r == n,
                None => self.parent(n).is_none(),
            },
            Pseudo::Empty => self.tree.children(n).is_empty(),
            Pseudo::FirstChild => index(&self.siblings(n)) == 1,
            Pseudo::LastChild => {
                let s = self.siblings(n);
                index(&s) == s.len() as i32
            }
            Pseudo::OnlyChild => self.siblings(n).len() == 1,
            Pseudo::NthChild(nth) => nth_holds(nth, index(&self.siblings(n))),
            Pseudo::NthLastChild(nth) => {
                let s = self.siblings(n);
                nth_holds(nth, s.len() as i32 - index(&s) + 1)
            }
            Pseudo::FirstOfType => index(&of_type(self.siblings(n))) == 1,
            Pseudo::LastOfType => {
                let s = of_type(self.siblings(n));
                index(&s) == s.len() as i32
            }
            Pseudo::OnlyOfType => of_type(self.siblings(n)).len() == 1,
            Pseudo::NthOfType(nth) => nth_holds(nth, index(&of_type(self.siblings(n)))),
            Pseudo::NthLastOfType(nth) => {
                let s = of_type(self.siblings(n));
                nth_holds(nth, s.len() as i32 - index(&s) + 1)
            }
            Pseudo::Not(list) => !sheet
                .sheet
                .selector_list(list)
                .iter()
                .any(|s| self.matches(sheet, s, n, deps)),
            Pseudo::Is(list) | Pseudo::Where(list) => sheet
                .sheet
                .selector_list(list)
                .iter()
                .any(|s| self.matches(sheet, s, n, deps)),
            Pseudo::Has(list) => sheet
                .sheet
                .selector_list(list)
                .iter()
                .any(|s| self.has(sheet, s, n, deps)),
        }
    }

    /// Whether an element related to `anchor` as `s`'s leading combinator says matches it.
    fn has(
        &self,
        sheet: &Sheet,
        s: &Selector,
        anchor: T::Node,
        deps: &mut Dependencies<T::Node>,
    ) -> bool {
        let last = s.compounds.len as usize - 1;
        let candidates = match s.leading.unwrap_or(Combinator::Descendant) {
            Combinator::NextSibling | Combinator::LaterSibling => {
                let sib = self.siblings(anchor);
                let k = sib.iter().position(|&x| x == anchor).unwrap_or(0);
                sib[k + 1..]
                    .iter()
                    .flat_map(|&x| self.subtree(x))
                    .collect::<Vec<_>>()
            }
            _ => {
                let mut all = self.subtree(anchor);
                all.remove(0);
                all
            }
        };
        candidates
            .into_iter()
            .any(|x| self.at(sheet, s, last, x, Some(anchor), deps))
    }

    /// `n` and everything under it, depth first.
    fn subtree(&self, n: T::Node) -> Vec<T::Node> {
        let mut out = Vec::new();
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            out.push(x);
            let mut children = self.tree.children(x);
            children.reverse();
            stack.extend(children);
        }
        out
    }
}

/// Whether an attribute's value (none when it has none) passes the test (none for `[name]`).
fn attribute(have: Option<&str>, test: Option<(AttributeOp, &str)>) -> bool {
    let Some(v) = have else { return false };
    match test {
        None => true,
        Some((op, want)) => match op {
            AttributeOp::Equals => v == want,
            AttributeOp::Includes => v.split(' ').any(|w| w == want),
            AttributeOp::DashMatch => v == want || v.starts_with(&format!("{want}-")),
            AttributeOp::Prefix => !want.is_empty() && v.starts_with(want),
            AttributeOp::Suffix => !want.is_empty() && v.ends_with(want),
            AttributeOp::Substring => !want.is_empty() && v.contains(want),
        },
    }
}

/// Whether 1-based position `i` is one of `an+b` for some n ≥ 0.
fn nth_holds(n: Nth, i: i32) -> bool {
    if i < 1 {
        return false;
    }
    if n.a == 0 {
        return i == n.b;
    }
    let d = i - n.b;
    d % n.a == 0 && d / n.a >= 0
}
