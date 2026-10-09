//! CSS for every SDK: stylesheets parsed into rules, selectors and
//! declarations, and a compiled form that loads without parsing.
//!
//! A declaration keeps its value as text, read as a typed value when it is
//! applied, so one sheet serves every target. A malformed rule is skipped and reported with its line and column;
//! the rest of the sheet still applies.
//!
//! A sheet is laid out in arenas. Every string is interned once into one
//! text buffer and named by an [`Atom`], so equal names are equal numbers.
//! Selectors, compounds, pseudo-classes, declarations and media are flat
//! vectors, and a node names its children by a [`Span`] of one of them. The
//! compiled form is those vectors as they are.

pub mod compiled;
pub mod json;
pub mod media;
mod parser;
mod tree;
pub mod value;

pub use media::{Compare, MediaEnvironment};
pub use parser::{Loader, parse, parse_selectors};
use std::collections::HashMap;
use std::ops::Range;

/// An interned string of a sheet: equal strings are equal atoms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Atom(pub u32);

/// A run of one of the sheet's arenas: where a node's children are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub fn range(self) -> Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Interned strings, kept end to end in one buffer.
#[derive(Clone, Debug, Default)]
pub struct Atoms {
    text: String,
    /// Each atom's byte range in `text`.
    spans: Vec<(u32, u32)>,
    /// Atoms by the hash of their text; another atom of the same hash is in `overflow`.
    index: HashMap<u64, u32>,
    overflow: Vec<u32>,
}

impl PartialEq for Atoms {
    fn eq(&self, other: &Self) -> bool {
        self.spans.len() == other.spans.len()
            && (0..self.spans.len() as u32).all(|i| self.get(Atom(i)) == other.get(Atom(i)))
    }
}

fn hash(s: &str) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(s)
}

impl Atoms {
    pub fn get(&self, a: Atom) -> &str {
        let (start, end) = self.spans[a.0 as usize];
        &self.text[start as usize..end as usize]
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The atom of `s`, adding it if it is new.
    pub fn intern(&mut self, s: &str) -> Atom {
        if let Some(a) = self.find(s) {
            return a;
        }
        let id = self.spans.len() as u32;
        let start = self.text.len() as u32;
        self.text.push_str(s);
        self.spans.push((start, self.text.len() as u32));
        self.remember(id);
        Atom(id)
    }

    /// Indexes the atom `id` by its text's hash; one whose hash is taken goes to `overflow`.
    fn remember(&mut self, id: u32) {
        match self.index.entry(hash(self.get(Atom(id)))) {
            std::collections::hash_map::Entry::Occupied(_) => self.overflow.push(id),
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(id);
            }
        }
    }

    /// The atom of `s` if the sheet has it.
    pub fn find(&self, s: &str) -> Option<Atom> {
        match self.index.get(&hash(s)) {
            Some(&i) if self.get(Atom(i)) == s => Some(Atom(i)),
            Some(_) => self
                .overflow
                .iter()
                .copied()
                .map(Atom)
                .find(|&a| self.get(a) == s),
            None => None,
        }
    }

    /// The buffer and each atom's byte range in it: the compiled form.
    pub(crate) fn parts(&self) -> (&str, &[(u32, u32)]) {
        (&self.text, &self.spans)
    }

    /// Atoms from a buffer and ranges, as `parts` gives them; none if a range is outside the buffer.
    pub(crate) fn from_parts(text: String, spans: Vec<(u32, u32)>) -> Option<Self> {
        let mut atoms = Atoms {
            text,
            spans: Vec::with_capacity(spans.len()),
            index: HashMap::new(),
            overflow: Vec::new(),
        };
        for (start, end) in spans {
            if start > end
                || end as usize > atoms.text.len()
                || !atoms.text.is_char_boundary(start as usize)
                || !atoms.text.is_char_boundary(end as usize)
            {
                return None;
            }
            let id = atoms.spans.len() as u32;
            atoms.spans.push((start, end));
            atoms.remember(id);
        }
        Some(atoms)
    }
}

/// How two compounds of a selector relate: `a b` any descendant, `a > b` a
/// child, `a + b` the next sibling, `a ~ b` any later sibling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

/// An `an+b` pattern, as `:nth-child()` takes: `odd` is 2n+1, `3` is 0n+3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nth {
    pub a: i32,
    pub b: i32,
}

/// The states a `State` pseudo-class may name.
pub const STATES: &[&str] = &[
    "hover",
    "active",
    "focus",
    "focus-visible",
    "focus-within",
    "disabled",
    "enabled",
    "checked",
    "indeterminate",
    "placeholder-shown",
    "valid",
    "invalid",
    "user-valid",
    "user-invalid",
    "required",
    "optional",
];

/// A pseudo-class of a compound selector. A selector list is a span of the sheet's selectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pseudo {
    /// An element state, one of [`STATES`].
    State(Atom),
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    NthChild(Nth),
    NthLastChild(Nth),
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthOfType(Nth),
    NthLastOfType(Nth),
    /// Matches when none of the selectors does.
    Not(Span),
    /// Matches when any of the selectors does.
    Is(Span),
    /// As `Is`, adding nothing to the specificity.
    Where(Span),
    /// Matches when an element relative to this one matches: `:has(> img)`.
    Has(Span),
}

/// How an attribute's value is compared: `=`, `~=`, `|=`, `^=`, `$=`, `*=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeOp {
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

impl AttributeOp {
    pub fn as_str(self) -> &'static str {
        match self {
            AttributeOp::Equals => "=",
            AttributeOp::Includes => "~=",
            AttributeOp::DashMatch => "|=",
            AttributeOp::Prefix => "^=",
            AttributeOp::Suffix => "$=",
            AttributeOp::Substring => "*=",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "=" => AttributeOp::Equals,
            "~=" => AttributeOp::Includes,
            "|=" => AttributeOp::DashMatch,
            "^=" => AttributeOp::Prefix,
            "$=" => AttributeOp::Suffix,
            "*=" => AttributeOp::Substring,
            _ => return None,
        })
    }
}

/// An attribute test: `[name]`, or `[name op "value"]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attribute {
    pub name: Atom,
    /// The comparison and the value it compares with; none for `[name]`.
    pub test: Option<(AttributeOp, Atom)>,
}

/// One compound selector: an element type or `*`, then any id, classes,
/// attributes and pseudo-classes it must have, all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Compound {
    /// The element type, or none for any.
    pub type_name: Option<Atom>,
    pub id: Option<Atom>,
    /// A span of the sheet's classes.
    pub classes: Span,
    /// A span of the sheet's attributes.
    pub attributes: Span,
    /// A span of the sheet's pseudos.
    pub pseudos: Span,
    /// A pseudo-element, `placeholder` for `::placeholder`.
    pub pseudo_element: Option<Atom>,
}

/// A complex selector: compounds joined by combinators, read right to left
/// when matching. Combinator `i` joins compound `i` to compound `i + 1`. In a
/// `:has()` argument the combinator before the first compound, if any, is
/// `leading`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selector {
    /// A span of the sheet's compounds.
    pub compounds: Span,
    /// A span of the sheet's combinators, one fewer than the compounds.
    pub combinators: Span,
    pub leading: Option<Combinator>,
    /// CSS's specificity as one number: ids × 10⁶, then classes, attributes
    /// and pseudo-classes × 10³, then types and pseudo-elements.
    pub specificity: u32,
}

/// One `property: value` of a rule: the value's text, trimmed, with any
/// `!important` removed and noted. A custom property keeps its `--` name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Declaration {
    pub name: Atom,
    pub value: Atom,
    pub important: bool,
    pub line: u32,
    pub column: u32,
}

/// One query of a list: an optional `not`, and features that must all hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaQuery {
    pub not: bool,
    /// A span of the sheet's features.
    pub features: Span,
}

/// A media feature. `Both` names two features of the sheet by index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaFeature {
    Width(Compare, f64),
    Height(Compare, f64),
    AspectRatio(Compare, f64),
    /// True for portrait.
    Orientation(bool),
    /// True for dark.
    ColorScheme(bool),
    /// A feature with a fixed answer here: `hover: hover` holds, `print` does not.
    Fixed(bool),
    /// Both bounds of a range written with two comparisons, `400px <= width <= 800px`.
    Both(u32, u32),
}

/// A style rule: what its selectors match takes its declarations, while its
/// media conditions hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    /// A span of the sheet's selectors.
    pub selectors: Span,
    /// A span of the sheet's declarations.
    pub declarations: Span,
    /// The `@media` query lists it is inside, each of which must hold: a
    /// span of the sheet's media lists; none outside any.
    pub media: Option<Span>,
    /// Its place among the sheet's rules, for ties in specificity.
    pub order: u32,
    pub line: u32,
}

/// One step of a `@keyframes`: the offsets it stands at, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    /// A span of the sheet's offsets.
    pub offsets: Span,
    /// A span of the sheet's declarations.
    pub declarations: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframes {
    pub name: Atom,
    /// A span of the sheet's frames.
    pub frames: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The construct was skipped.
    Error,
    /// Parsed, but something in it is not supported or has no effect.
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub line: u32,
    pub column: u32,
    /// The file it is in, when it is not the sheet's own: one it imports.
    pub file: Option<String>,
}

/// A parsed stylesheet, in arenas: its rules in source order, the custom
/// properties its `:root` rules declare, its `@keyframes`, and what went
/// wrong reading it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub atoms: Atoms,
    pub rules: Vec<Rule>,
    pub selectors: Vec<Selector>,
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    pub classes: Vec<Atom>,
    pub attributes: Vec<Attribute>,
    pub pseudos: Vec<Pseudo>,
    pub declarations: Vec<Declaration>,
    /// Each a span of `queries`: one `@media` list.
    pub media_lists: Vec<Span>,
    pub queries: Vec<MediaQuery>,
    pub features: Vec<MediaFeature>,
    /// `:root`'s custom properties, by name without the `--`, in source
    /// order; a later one of a name replaces the earlier in place.
    pub variables: Vec<(Atom, Atom)>,
    /// In the order they were read; a later one of a name replaces the earlier.
    pub keyframes: Vec<Keyframes>,
    pub frames: Vec<Keyframe>,
    pub offsets: Vec<f64>,
    /// The files it imported, directly or through another, in the order read.
    pub imports: Vec<Atom>,
    pub diagnostics: Vec<Diagnostic>,
}

fn span_of(start: usize, end: usize) -> Span {
    Span {
        start: start as u32,
        len: (end - start) as u32,
    }
}

impl Stylesheet {
    pub fn str(&self, a: Atom) -> &str {
        self.atoms.get(a)
    }

    pub fn rule_selectors(&self, r: &Rule) -> &[Selector] {
        &self.selectors[r.selectors.range()]
    }

    pub fn rule_declarations(&self, r: &Rule) -> &[Declaration] {
        &self.declarations[r.declarations.range()]
    }

    pub fn selector_list(&self, s: Span) -> &[Selector] {
        &self.selectors[s.range()]
    }

    pub fn selector_compounds(&self, s: &Selector) -> &[Compound] {
        &self.compounds[s.compounds.range()]
    }

    pub fn selector_combinators(&self, s: &Selector) -> &[Combinator] {
        &self.combinators[s.combinators.range()]
    }

    /// The compound a selector applies its declarations to: its last.
    pub fn subject(&self, s: &Selector) -> &Compound {
        &self.compounds[(s.compounds.start + s.compounds.len - 1) as usize]
    }

    pub fn compound_classes(&self, c: &Compound) -> &[Atom] {
        &self.classes[c.classes.range()]
    }

    pub fn compound_attributes(&self, c: &Compound) -> &[Attribute] {
        &self.attributes[c.attributes.range()]
    }

    pub fn compound_pseudos(&self, c: &Compound) -> &[Pseudo] {
        &self.pseudos[c.pseudos.range()]
    }

    /// A rule's `@media` lists; empty outside any.
    pub fn rule_media(&self, r: &Rule) -> &[Span] {
        r.media.map_or(&[][..], |m| &self.media_lists[m.range()])
    }

    pub fn list_queries(&self, list: Span) -> &[MediaQuery] {
        &self.queries[list.range()]
    }

    pub fn query_features(&self, q: &MediaQuery) -> &[MediaFeature] {
        &self.features[q.features.range()]
    }

    pub fn keyframe_list(&self, k: &Keyframes) -> &[Keyframe] {
        &self.frames[k.frames.range()]
    }

    pub fn keyframe_offsets(&self, f: &Keyframe) -> &[f64] {
        &self.offsets[f.offsets.range()]
    }

    pub fn keyframe_declarations(&self, f: &Keyframe) -> &[Declaration] {
        &self.declarations[f.declarations.range()]
    }

    pub fn variable(&self, name: &str) -> Option<&str> {
        let a = self.atoms.find(name)?;
        self.variables
            .iter()
            .find(|(k, _)| *k == a)
            .map(|(_, v)| self.str(*v))
    }

    pub fn keyframes_named(&self, name: &str) -> Option<&Keyframes> {
        let a = self.atoms.find(name)?;
        self.keyframes.iter().find(|k| k.name == a)
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Whether every `@media` list a rule is inside holds.
    pub fn media_holds(&self, r: &Rule, env: &MediaEnvironment) -> bool {
        self.rule_media(r).iter().all(|&list| {
            self.list_queries(list).iter().any(|q| {
                self.query_features(q)
                    .iter()
                    .all(|f| self.feature_holds(f, env))
                    != q.not
            })
        })
    }

    fn feature_holds(&self, f: &MediaFeature, env: &MediaEnvironment) -> bool {
        let cmp = |op: Compare, a: f64, b: f64| match op {
            Compare::Eq => (a - b).abs() < 0.001,
            Compare::Lt => a < b,
            Compare::Le => a <= b,
            Compare::Gt => a > b,
            Compare::Ge => a >= b,
        };
        match *f {
            MediaFeature::Width(op, px) => cmp(op, env.width, px),
            MediaFeature::Height(op, px) => cmp(op, env.height, px),
            MediaFeature::AspectRatio(op, r) => {
                env.height > 0.0 && cmp(op, env.width / env.height, r)
            }
            MediaFeature::Orientation(portrait) => (env.height >= env.width) == portrait,
            MediaFeature::ColorScheme(dark) => env.dark == dark,
            MediaFeature::Fixed(holds) => holds,
            MediaFeature::Both(a, b) => {
                self.feature_holds(&self.features[a as usize], env)
                    && self.feature_holds(&self.features[b as usize], env)
            }
        }
    }

    /// The errors and warnings, one a line, as `file:line:column: message`.
    pub fn report(&self, file: Option<&str>) -> String {
        self.diagnostics
            .iter()
            .map(|d| {
                let at = d
                    .file
                    .as_deref()
                    .or(file)
                    .map(|f| format!("{f}:"))
                    .unwrap_or_default();
                let kind = if d.severity == Severity::Error {
                    "error"
                } else {
                    "warning"
                };
                format!("{at}{}:{}: {kind}: {}", d.line, d.column, d.message)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The CSS class names any of its selectors mentions, sorted.
    pub fn class_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .classes
            .iter()
            .map(|&a| self.str(a).to_string())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// A selector as CSS text, as written canonically.
    pub fn selector_text(&self, s: &Selector) -> String {
        let mut out = String::new();
        if let Some(l) = s.leading {
            out += combinator_str(l);
            out.push(' ');
        }
        let combinators = self.selector_combinators(s);
        for (i, c) in self.selector_compounds(s).iter().enumerate() {
            out += &self.compound_text(c);
            if let Some(&k) = combinators.get(i) {
                match k {
                    Combinator::Descendant => out.push(' '),
                    other => out += &format!(" {} ", combinator_str(other)),
                }
            }
        }
        out
    }

    fn compound_text(&self, c: &Compound) -> String {
        let mut out = c
            .type_name
            .map(|a| self.str(a).to_string())
            .unwrap_or_default();
        if let Some(id) = c.id {
            out += &format!("#{}", self.str(id));
        }
        for &cl in self.compound_classes(c) {
            out += &format!(".{}", self.str(cl));
        }
        for a in self.compound_attributes(c) {
            out += &match a.test {
                None => format!("[{}]", self.str(a.name)),
                Some((op, v)) => {
                    format!("[{}{}\"{}\"]", self.str(a.name), op.as_str(), self.str(v))
                }
            };
        }
        for p in self.compound_pseudos(c) {
            out += &self.pseudo_text(p);
        }
        if let Some(pe) = c.pseudo_element {
            out += &format!("::{}", self.str(pe));
        }
        if out.is_empty() { "*".into() } else { out }
    }

    fn pseudo_text(&self, p: &Pseudo) -> String {
        let nth = |n: &Nth| {
            if n.a == 0 {
                n.b.to_string()
            } else {
                format!("{}n{}{}", n.a, if n.b < 0 { "" } else { "+" }, n.b)
            }
        };
        let list = |s: Span| {
            self.selector_list(s)
                .iter()
                .map(|x| self.selector_text(x))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match p {
            Pseudo::State(name) => format!(":{}", self.str(*name)),
            Pseudo::Root => ":root".into(),
            Pseudo::Empty => ":empty".into(),
            Pseudo::FirstChild => ":first-child".into(),
            Pseudo::LastChild => ":last-child".into(),
            Pseudo::OnlyChild => ":only-child".into(),
            Pseudo::NthChild(n) => format!(":nth-child({})", nth(n)),
            Pseudo::NthLastChild(n) => format!(":nth-last-child({})", nth(n)),
            Pseudo::FirstOfType => ":first-of-type".into(),
            Pseudo::LastOfType => ":last-of-type".into(),
            Pseudo::OnlyOfType => ":only-of-type".into(),
            Pseudo::NthOfType(n) => format!(":nth-of-type({})", nth(n)),
            Pseudo::NthLastOfType(n) => format!(":nth-last-of-type({})", nth(n)),
            Pseudo::Not(s) => format!(":not({})", list(*s)),
            Pseudo::Is(s) => format!(":is({})", list(*s)),
            Pseudo::Where(s) => format!(":where({})", list(*s)),
            Pseudo::Has(s) => format!(":has({})", list(*s)),
        }
    }

    // --- Building ---

    /// Lays out a selector list read by the parser: everything a selector
    /// nests first, so each list, each selector's compounds and each
    /// compound's parts stay contiguous.
    pub(crate) fn push_selectors(&mut self, list: &[tree::Selector]) -> Span {
        let laid: Vec<Selector> = list.iter().map(|s| self.lay_selector(s)).collect();
        let start = self.selectors.len();
        self.selectors.extend(laid);
        span_of(start, self.selectors.len())
    }

    fn lay_selector(&mut self, s: &tree::Selector) -> Selector {
        let laid: Vec<Compound> = s.compounds.iter().map(|c| self.lay_compound(c)).collect();
        let start = self.compounds.len();
        self.compounds.extend(laid);
        let compounds = span_of(start, self.compounds.len());
        let start = self.combinators.len();
        self.combinators.extend_from_slice(&s.combinators);
        let combinators = span_of(start, self.combinators.len());
        Selector {
            compounds,
            combinators,
            leading: s.leading,
            specificity: s.specificity(),
        }
    }

    fn lay_compound(&mut self, c: &tree::Compound) -> Compound {
        let pseudos: Vec<Pseudo> = c
            .pseudos
            .iter()
            .map(|p| match p {
                tree::Pseudo::State(s) => Pseudo::State(self.atoms.intern(s)),
                tree::Pseudo::Root => Pseudo::Root,
                tree::Pseudo::Empty => Pseudo::Empty,
                tree::Pseudo::FirstChild => Pseudo::FirstChild,
                tree::Pseudo::LastChild => Pseudo::LastChild,
                tree::Pseudo::OnlyChild => Pseudo::OnlyChild,
                tree::Pseudo::NthChild(n) => Pseudo::NthChild(*n),
                tree::Pseudo::NthLastChild(n) => Pseudo::NthLastChild(*n),
                tree::Pseudo::FirstOfType => Pseudo::FirstOfType,
                tree::Pseudo::LastOfType => Pseudo::LastOfType,
                tree::Pseudo::OnlyOfType => Pseudo::OnlyOfType,
                tree::Pseudo::NthOfType(n) => Pseudo::NthOfType(*n),
                tree::Pseudo::NthLastOfType(n) => Pseudo::NthLastOfType(*n),
                tree::Pseudo::Not(s) => Pseudo::Not(self.push_selectors(s)),
                tree::Pseudo::Is(s) => Pseudo::Is(self.push_selectors(s)),
                tree::Pseudo::Where(s) => Pseudo::Where(self.push_selectors(s)),
                tree::Pseudo::Has(s) => Pseudo::Has(self.push_selectors(s)),
            })
            .collect();
        let type_name = c.type_name.as_deref().map(|s| self.atoms.intern(s));
        let id = c.id.as_deref().map(|s| self.atoms.intern(s));
        let pseudo_element = c.pseudo_element.as_deref().map(|s| self.atoms.intern(s));
        let start = self.classes.len();
        for x in &c.classes {
            let a = self.atoms.intern(x);
            self.classes.push(a);
        }
        let classes = span_of(start, self.classes.len());
        let start = self.attributes.len();
        for a in &c.attributes {
            let name = self.atoms.intern(&a.name);
            let test = match (&a.op, &a.value) {
                (Some(op), Some(v)) => Some((
                    AttributeOp::parse(op).unwrap_or(AttributeOp::Equals),
                    self.atoms.intern(v),
                )),
                _ => None,
            };
            self.attributes.push(Attribute { name, test });
        }
        let attributes = span_of(start, self.attributes.len());
        let start = self.pseudos.len();
        self.pseudos.extend(pseudos);
        let pseudos = span_of(start, self.pseudos.len());
        Compound {
            type_name,
            id,
            classes,
            attributes,
            pseudos,
            pseudo_element,
        }
    }

    pub(crate) fn push_declarations(&mut self, list: &[tree::Declaration]) -> Span {
        let start = self.declarations.len();
        for d in list {
            let name = self.atoms.intern(&d.name);
            let value = self.atoms.intern(&d.value);
            self.declarations.push(Declaration {
                name,
                value,
                important: d.important,
                line: d.line,
                column: d.column,
            });
        }
        span_of(start, self.declarations.len())
    }

    pub(crate) fn push_media(&mut self, lists: &[Vec<media::MediaQuery>]) -> Span {
        let laid: Vec<Span> = lists
            .iter()
            .map(|list| {
                let queries: Vec<MediaQuery> = list
                    .iter()
                    .map(|q| {
                        let features: Vec<MediaFeature> =
                            q.features.iter().map(|f| self.lay_feature(f)).collect();
                        let start = self.features.len();
                        self.features.extend(features);
                        MediaQuery {
                            not: q.not,
                            features: span_of(start, self.features.len()),
                        }
                    })
                    .collect();
                let start = self.queries.len();
                self.queries.extend(queries);
                span_of(start, self.queries.len())
            })
            .collect();
        let start = self.media_lists.len();
        self.media_lists.extend(laid);
        span_of(start, self.media_lists.len())
    }

    fn lay_feature(&mut self, f: &media::MediaFeature) -> MediaFeature {
        match f {
            media::MediaFeature::Width(op, v) => MediaFeature::Width(*op, *v),
            media::MediaFeature::Height(op, v) => MediaFeature::Height(*op, *v),
            media::MediaFeature::AspectRatio(op, v) => MediaFeature::AspectRatio(*op, *v),
            media::MediaFeature::Orientation(b) => MediaFeature::Orientation(*b),
            media::MediaFeature::ColorScheme(b) => MediaFeature::ColorScheme(*b),
            media::MediaFeature::Fixed(b) => MediaFeature::Fixed(*b),
            media::MediaFeature::Both(a, b) => {
                let a = self.lay_feature(a);
                let b = self.lay_feature(b);
                self.features.push(a);
                self.features.push(b);
                let n = self.features.len() as u32;
                MediaFeature::Both(n - 2, n - 1)
            }
        }
    }

    pub(crate) fn set_variable(&mut self, name: &str, value: &str) {
        let name = self.atoms.intern(name);
        let value = self.atoms.intern(value);
        match self.variables.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.variables.push((name, value)),
        }
    }

    pub(crate) fn set_keyframes(
        &mut self,
        name: &str,
        frames: &[(Vec<f64>, Vec<tree::Declaration>)],
    ) {
        let laid: Vec<Keyframe> = frames
            .iter()
            .map(|(offsets, declarations)| {
                let start = self.offsets.len();
                self.offsets.extend_from_slice(offsets);
                let offsets = span_of(start, self.offsets.len());
                Keyframe {
                    offsets,
                    declarations: self.push_declarations(declarations),
                }
            })
            .collect();
        let start = self.frames.len();
        self.frames.extend(laid);
        let k = Keyframes {
            name: self.atoms.intern(name),
            frames: span_of(start, self.frames.len()),
        };
        match self.keyframes.iter_mut().find(|x| x.name == k.name) {
            Some(slot) => *slot = k,
            None => self.keyframes.push(k),
        }
    }
}

fn combinator_str(c: Combinator) -> &'static str {
    match c {
        Combinator::Descendant => "",
        Combinator::Child => ">",
        Combinator::NextSibling => "+",
        Combinator::LaterSibling => "~",
    }
}

#[cfg(test)]
mod tests;
