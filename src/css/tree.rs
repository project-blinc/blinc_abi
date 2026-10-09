//! The owned form a stylesheet is read into before it is laid out in the
//! sheet's arenas: what the parser builds and composes as text.

use super::{Combinator, Nth};

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

impl Selector {
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
