//! CSS for every SDK: stylesheets parsed into rules, selectors and
//! declarations, and a compiled form that loads without parsing.
//!
//! The model is ashui's (`ashui.css`): a declaration keeps its value as
//! text, read as a typed value when it is applied, so one sheet serves every
//! target. A malformed rule is skipped and reported with its line and column;
//! the rest of the sheet still applies.

pub mod compiled;
pub mod json;
pub mod media;
mod parser;
pub mod value;

pub use media::{Compare, MediaFeature, MediaQuery};
pub use parser::{parse, parse_selectors};

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

/// A pseudo-class of a compound selector.
#[derive(Clone, Debug, PartialEq)]
pub enum Pseudo {
    /// An element state, one of [`STATES`].
    State(String),
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
    Not(Vec<Selector>),
    /// Matches when any of the selectors does.
    Is(Vec<Selector>),
    /// As `Is`, adding nothing to the specificity.
    Where(Vec<Selector>),
    /// Matches when an element relative to this one matches: `:has(> img)`.
    Has(Vec<Selector>),
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

/// An attribute test: `[name]`, or `[name op "value"]` with op one of `=`,
/// `~=`, `|=`, `^=`, `$=`, `*=`.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub op: Option<String>,
    pub value: Option<String>,
}

/// One compound selector: an element type or `*`, then any id, classes,
/// attributes and pseudo-classes it must have, all of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    /// The element type, or none for any.
    pub type_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attributes: Vec<Attribute>,
    pub pseudos: Vec<Pseudo>,
    /// A pseudo-element, `placeholder` for `::placeholder`.
    pub pseudo_element: Option<String>,
}

/// A complex selector: compounds joined by combinators, read right to left
/// when matching. `combinators[i]` joins `compounds[i]` to `compounds[i + 1]`.
/// In a `:has()` argument the combinator before the first compound, if any,
/// is `leading`.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    pub leading: Option<Combinator>,
}

/// One `property: value` of a rule: the value's text, trimmed, with any
/// `!important` removed and noted. A custom property keeps its `--` name.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
    pub line: u32,
    pub column: u32,
}

/// A style rule: what its selectors match takes its declarations, while its
/// media conditions hold.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// The `@media` query lists it is inside, each of which must hold.
    pub media: Option<Vec<Vec<MediaQuery>>>,
    /// Its place among the sheet's rules, for ties in specificity.
    pub order: u32,
    pub line: u32,
}

/// One step of a `@keyframes`: the offsets it stands at, 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub offsets: Vec<f64>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    pub name: String,
    pub frames: Vec<Keyframe>,
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

/// A parsed stylesheet: its style rules in source order, the custom
/// properties its `:root` rules declare, its `@keyframes`, and what went
/// wrong reading it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    /// `:root`'s custom properties, by name without the `--`, in source
    /// order; a later one of a name replaces the earlier in place.
    pub variables: Vec<(String, String)>,
    /// In the order they were read; a later one of a name replaces the earlier.
    pub keyframes: Vec<Keyframes>,
    /// The files it imported, directly or through another, in the order read.
    pub imports: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Stylesheet {
    pub fn variable(&self, name: &str) -> Option<&str> {
        self.variables
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn keyframes(&self, name: &str) -> Option<&Keyframes> {
        self.keyframes.iter().find(|k| k.name == name)
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
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
        fn visit(selectors: &[Selector], out: &mut Vec<String>) {
            for s in selectors {
                for c in &s.compounds {
                    out.extend(c.classes.iter().cloned());
                    for p in &c.pseudos {
                        if let Pseudo::Not(inner)
                        | Pseudo::Is(inner)
                        | Pseudo::Where(inner)
                        | Pseudo::Has(inner) = p
                        {
                            visit(inner, out);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for r in &self.rules {
            visit(&r.selectors, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }

    pub(crate) fn set_variable(&mut self, name: String, value: String) {
        match self.variables.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.variables.push((name, value)),
        }
    }

    pub(crate) fn set_keyframes(&mut self, k: Keyframes) {
        match self.keyframes.iter_mut().find(|x| x.name == k.name) {
            Some(slot) => *slot = k,
            None => self.keyframes.push(k),
        }
    }
}

impl Selector {
    /// The compound the selector applies its declarations to: its last.
    pub fn subject(&self) -> &Compound {
        &self.compounds[self.compounds.len() - 1]
    }

    /// CSS's specificity as one number: ids × 10⁶, then classes, attributes
    /// and pseudo-classes × 10³, then types and pseudo-elements. `:is`,
    /// `:not` and `:has` count their most specific argument; `:where` none.
    pub fn specificity(&self) -> u32 {
        self.compounds.iter().map(compound_specificity).sum()
    }
}

fn compound_specificity(c: &Compound) -> u32 {
    let mut n = 0;
    if c.id.is_some() {
        n += 1_000_000;
    }
    n += 1000 * (c.classes.len() + c.attributes.len()) as u32;
    if c.type_name.is_some() {
        n += 1;
    }
    if c.pseudo_element.is_some() {
        n += 1;
    }
    for p in &c.pseudos {
        n += match p {
            Pseudo::Not(s) | Pseudo::Is(s) | Pseudo::Has(s) => {
                s.iter().map(Selector::specificity).max().unwrap_or(0)
            }
            Pseudo::Where(_) => 0,
            _ => 1000,
        };
    }
    n
}

fn combinator_str(c: Combinator) -> &'static str {
    match c {
        Combinator::Descendant => "",
        Combinator::Child => ">",
        Combinator::NextSibling => "+",
        Combinator::LaterSibling => "~",
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(l) = self.leading {
            write!(f, "{} ", combinator_str(l))?;
        }
        for (i, c) in self.compounds.iter().enumerate() {
            write!(f, "{c}")?;
            if let Some(&k) = self.combinators.get(i) {
                match k {
                    Combinator::Descendant => write!(f, " ")?,
                    other => write!(f, " {} ", combinator_str(other))?,
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for Compound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = self.type_name.clone().unwrap_or_default();
        if let Some(id) = &self.id {
            out += &format!("#{id}");
        }
        for c in &self.classes {
            out += &format!(".{c}");
        }
        for a in &self.attributes {
            out += &match &a.op {
                None => format!("[{}]", a.name),
                Some(op) => format!("[{}{op}\"{}\"]", a.name, a.value.as_deref().unwrap_or("")),
            };
        }
        for p in &self.pseudos {
            out += &pseudo_str(p);
        }
        if let Some(pe) = &self.pseudo_element {
            out += &format!("::{pe}");
        }
        f.write_str(if out.is_empty() { "*" } else { &out })
    }
}

fn pseudo_str(p: &Pseudo) -> String {
    let nth = |n: &Nth| {
        if n.a == 0 {
            n.b.to_string()
        } else {
            format!("{}n{}{}", n.a, if n.b < 0 { "" } else { "+" }, n.b)
        }
    };
    let list = |s: &[Selector]| {
        s.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match p {
        Pseudo::State(name) => format!(":{name}"),
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
        Pseudo::Not(s) => format!(":not({})", list(s)),
        Pseudo::Is(s) => format!(":is({})", list(s)),
        Pseudo::Where(s) => format!(":where({})", list(s)),
        Pseudo::Has(s) => format!(":has({})", list(s)),
    }
}

#[cfg(test)]
mod tests;
