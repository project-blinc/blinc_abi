//! `calc()`, `min()`, `max()` and `clamp()` of lengths.
//!
//! A value is a number of pixels plus a fraction of something the layout knows only later, a
//! percentage. Layout cannot add the two, so a value that has both is an error where a length
//! is wanted; lengths in px, em, rem and the viewport units are all pixels by the time they are
//! read, so `calc(100vw - 2rem)` and `clamp(14px, 2vw, 20px)` are pixels, and a percentage alone,
//! `calc(100% / 3)`, is a fraction.

/// A length: pixels plus a fraction of the percentage's basis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length {
    pub px: f64,
    pub percent: f64,
}

/// What an expression's term is: a plain number, which scales a length, or a length.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Term {
    Number(f64),
    Length(Length),
}

/// How a unit's number is read: as pixels, or as a fraction for `%`; none for a unit that is not one.
pub type Unit<'a> = &'a dyn Fn(f64, &str) -> Option<Length>;

/// Whether `text` starts a math function, including malformed expressions for diagnostics.
pub fn is_function(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    ["calc(", "min(", "max(", "clamp("]
        .iter()
        .any(|f| lower.starts_with(f))
}

/// The value of the function `text`, with `unit` reading each number's unit.
pub fn evaluate(text: &str, unit: Unit<'_>) -> Result<Length, String> {
    let mut parser = Parser {
        text: text.trim().as_bytes(),
        at: 0,
        unit,
        depth: 0,
    };
    let term = parser.expression()?;
    parser.space();
    if parser.at != parser.text.len() {
        return Err(format!("unexpected \"{}\" in {text}", parser.rest()));
    }
    match term {
        Term::Length(l) => Ok(l),
        Term::Number(_) => Err(format!("{text} is a number, not a length")),
    }
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    unit: Unit<'a>,
    depth: usize,
}

impl Term {
    fn finite(self) -> Result<Self, String> {
        let finite = match self {
            Self::Number(n) => n.is_finite(),
            Self::Length(l) => l.px.is_finite() && l.percent.is_finite(),
        };
        if finite {
            Ok(self)
        } else {
            Err("calc() produced a non-finite value".into())
        }
    }
}

impl Parser<'_> {
    fn rest(&self) -> String {
        String::from_utf8_lossy(&self.text[self.at.min(self.text.len())..]).into_owned()
    }

    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.text.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// `term (('+' | '-') term)*`
    fn expression(&mut self) -> Result<Term, String> {
        let mut left = self.product()?;
        loop {
            self.space();
            let op = match self.text.get(self.at) {
                Some(b'+') => b'+',
                Some(b'-') => b'-',
                _ => return Ok(left),
            };
            if !self
                .text
                .get(self.at.wrapping_sub(1))
                .is_some_and(u8::is_ascii_whitespace)
                || !self
                    .text
                    .get(self.at + 1)
                    .is_some_and(u8::is_ascii_whitespace)
            {
                return Err("calc() needs whitespace around + and -".into());
            }
            self.at += 1;
            let right = self.product()?;
            left = match (left, right) {
                (Term::Number(a), Term::Number(b)) => {
                    Term::Number(if op == b'+' { a + b } else { a - b })
                }
                (Term::Length(a), Term::Length(b)) => {
                    let sign = if op == b'+' { 1.0 } else { -1.0 };
                    Term::Length(Length {
                        px: a.px + sign * b.px,
                        percent: a.percent + sign * b.percent,
                    })
                }
                _ => return Err("a length cannot be added to a number".into()),
            }
            .finite()?;
        }
    }

    /// `factor (('*' | '/') factor)*`
    fn product(&mut self) -> Result<Term, String> {
        let mut left = self.factor()?;
        loop {
            self.space();
            let op = match self.text.get(self.at) {
                Some(b'*') => b'*',
                Some(b'/') => b'/',
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.factor()?;
            left = match (op, left, right) {
                (b'*', Term::Number(a), Term::Number(b)) => Term::Number(a * b),
                (b'*', Term::Length(l), Term::Number(n))
                | (b'*', Term::Number(n), Term::Length(l)) => Term::Length(Length {
                    px: l.px * n,
                    percent: l.percent * n,
                }),
                (b'/', Term::Number(a), Term::Number(b)) if b != 0.0 => Term::Number(a / b),
                (b'/', Term::Length(l), Term::Number(n)) if n != 0.0 => Term::Length(Length {
                    px: l.px / n,
                    percent: l.percent / n,
                }),
                (b'/', _, Term::Number(_)) => return Err("division by zero".into()),
                (b'*', Term::Length(_), Term::Length(_)) => {
                    return Err("two lengths cannot be multiplied".into());
                }
                _ => return Err("only a number can divide".into()),
            }
            .finite()?;
        }
    }

    fn factor(&mut self) -> Result<Term, String> {
        self.space();
        match self.text.get(self.at) {
            None => Err("an expression is cut short".into()),
            Some(b'(') => {
                self.at += 1;
                self.descend(|p| {
                    let inner = p.expression()?;
                    if !p.eat(b')') {
                        return Err("a parenthesis is not closed".into());
                    }
                    Ok(inner)
                })
            }
            Some(b'-') | Some(b'+') | Some(b'.') | Some(b'0'..=b'9') => self.number(),
            Some(c) if c.is_ascii_alphabetic() => self.function(),
            Some(_) => Err(format!("unexpected \"{}\"", self.rest())),
        }
    }

    fn descend<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if self.depth >= 16 {
            return Err("calc() is nested too deeply".into());
        }
        self.depth += 1;
        let out = f(self);
        self.depth -= 1;
        out
    }

    fn number(&mut self) -> Result<Term, String> {
        let start = self.at;
        if matches!(self.text.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        while self
            .text
            .get(self.at)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
        {
            self.at += 1;
        }
        // An exponent, as `1e3` is.
        if matches!(self.text.get(self.at), Some(b'e' | b'E'))
            && self
                .text
                .get(self.at + 1)
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-'))
        {
            self.at += 2;
            while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
        }
        let number: f64 = std::str::from_utf8(&self.text[start..self.at])
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or_else(|| format!("expected a number, not \"{}\"", self.rest()))?;
        let unit_start = self.at;
        if self.text.get(self.at) == Some(&b'%') {
            self.at += 1;
        } else {
            while self
                .text
                .get(self.at)
                .is_some_and(|c| c.is_ascii_alphabetic())
            {
                self.at += 1;
            }
        }
        let unit = std::str::from_utf8(&self.text[unit_start..self.at])
            .unwrap_or_default()
            .to_ascii_lowercase();
        if unit.is_empty() {
            return Term::Number(number).finite();
        }
        (self.unit)(number, &unit)
            .map(Term::Length)
            .ok_or_else(|| format!("\"{unit}\" is not a unit calc() reads here"))?
            .finite()
    }

    fn function(&mut self) -> Result<Term, String> {
        let start = self.at;
        while self
            .text
            .get(self.at)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
        {
            self.at += 1;
        }
        let name = String::from_utf8_lossy(&self.text[start..self.at]).to_ascii_lowercase();
        if !self.eat(b'(') {
            return Err(format!("unexpected \"{name}\""));
        }
        self.descend(|p| {
            let mut args = vec![p.expression()?];
            while p.eat(b',') {
                args.push(p.expression()?);
            }
            if !p.eat(b')') {
                return Err(format!("{name}( is not closed"));
            }
            match (name.as_str(), args.as_slice()) {
                ("calc", [one]) => Ok(*one),
                ("min", list) if !list.is_empty() => pick(list, f64::min),
                ("max", list) if !list.is_empty() => pick(list, f64::max),
                ("clamp", [low, value, high]) => {
                    // The value held between the bounds, the lower bound winning if they cross.
                    let upper = pick(&[*value, *high], f64::min)?;
                    pick(&[*low, upper], f64::max)
                }
                ("calc" | "min" | "max" | "clamp", _) => {
                    Err(format!("{name}() takes a different number of values"))
                }
                _ => Err(format!("{name}() is not a function calc() reads")),
            }
        })
    }
}

/// The smallest or largest of `terms` by `pick`: of numbers, or of lengths that can be compared,
/// which are all pixels or all one percentage, as a percentage cannot be set against pixels.
fn pick(terms: &[Term], pick: fn(f64, f64) -> f64) -> Result<Term, String> {
    let mut lengths = Vec::new();
    let mut numbers = Vec::new();
    for t in terms {
        match t {
            Term::Length(l) => lengths.push(*l),
            Term::Number(n) => numbers.push(*n),
        }
    }
    match (lengths.is_empty(), numbers.is_empty()) {
        (true, _) => Ok(Term::Number(
            numbers.into_iter().reduce(pick).unwrap_or(0.0),
        )),
        (false, true) => {
            if lengths.iter().all(|l| l.percent == 0.0) {
                Ok(Term::Length(Length {
                    px: lengths.iter().map(|l| l.px).reduce(pick).unwrap_or(0.0),
                    percent: 0.0,
                }))
            } else if lengths.iter().all(|l| l.px == 0.0) {
                Ok(Term::Length(Length {
                    px: 0.0,
                    percent: lengths
                        .iter()
                        .map(|l| l.percent)
                        .reduce(pick)
                        .unwrap_or(0.0),
                }))
            } else {
                Err("min(), max() and clamp() cannot compare a percentage with a length".into())
            }
        }
        _ => Err("a number and a length cannot be compared".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: f64, name: &str) -> Option<Length> {
        let px = |px| Some(Length { px, percent: 0.0 });
        match name {
            "px" => px(v),
            "em" => px(v * 16.0),
            "rem" => px(v * 10.0),
            "vw" => px(v / 100.0 * 800.0),
            "%" => Some(Length {
                px: 0.0,
                percent: v / 100.0,
            }),
            _ => None,
        }
    }

    fn px(text: &str) -> Result<f64, String> {
        evaluate(text, &unit).map(|l| {
            assert_eq!(l.percent, 0.0, "{text}");
            l.px
        })
    }

    #[test]
    fn arithmetic_of_lengths_in_any_unit_is_pixels() {
        assert_eq!(px("calc(1px + 2px)"), Ok(3.0));
        assert_eq!(px("calc(100vw - 2rem)"), Ok(780.0));
        assert_eq!(px("calc(1em + 4px)"), Ok(20.0));
        assert_eq!(px("calc(10px * 3)"), Ok(30.0));
        assert_eq!(px("calc(3 * 10px)"), Ok(30.0));
        assert_eq!(px("calc(100px / 4)"), Ok(25.0));
        assert_eq!(px("calc(2px + 3px * 2)"), Ok(8.0), "multiplication first");
        assert_eq!(px("calc((2px + 3px) * 2)"), Ok(10.0));
        assert_eq!(px("CALC( 5PX - 1px )"), Ok(4.0));
        assert_eq!(px("calc(-1 * 8px)"), Ok(-8.0));
        assert_eq!(px("calc(calc(1px + 1px) + 1px)"), Ok(3.0), "nested");
        assert_eq!(px("calc(1.5px*2)"), Ok(3.0), "no spaces");
    }

    #[test]
    fn min_max_and_clamp_pick_among_lengths() {
        assert_eq!(px("min(10px, 2rem, 30px)"), Ok(10.0));
        assert_eq!(px("max(10px, 1rem)"), Ok(10.0));
        assert_eq!(px("clamp(14px, 2vw, 20px)"), Ok(16.0));
        assert_eq!(px("clamp(14px, 1vw, 20px)"), Ok(14.0), "held to the least");
        assert_eq!(px("clamp(14px, 5vw, 20px)"), Ok(20.0), "and the most");
        assert_eq!(
            px("clamp(30px, 1px, 20px)"),
            Ok(30.0),
            "crossed bounds: the lower"
        );
        assert_eq!(px("calc(min(10px, 2px) + max(1px, 3px))"), Ok(5.0));
        assert_eq!(px("min(calc(1px + 1px), 5px)"), Ok(2.0));
    }

    #[test]
    fn a_percentage_alone_is_a_fraction() {
        let l = evaluate("calc(100% / 3)", &unit).unwrap();
        assert!((l.percent - 1.0 / 3.0).abs() < 1e-9 && l.px == 0.0);
        let l = evaluate("calc(50% + 25%)", &unit).unwrap();
        assert!((l.percent - 0.75).abs() < 1e-9);
        let l = evaluate("min(50%, 20%)", &unit).unwrap();
        assert!((l.percent - 0.2).abs() < 1e-9);
        // Mixed, the value has both, and the caller decides it cannot take it.
        let l = evaluate("calc(100% - 32px)", &unit).unwrap();
        assert_eq!((l.px, l.percent), (-32.0, 1.0));
        assert!(evaluate("min(50%, 20px)", &unit).is_err());
    }

    #[test]
    fn what_is_not_an_expression_is_an_error() {
        for bad in [
            "calc()",
            "calc(1px +)",
            "calc(1px + 2)",
            "calc(1px * 2px)",
            "calc(2 / 1px)",
            "calc(1px / 0)",
            "calc(1px",
            "calc(1px) 2px",
            "calc(1foo)",
            "calc(var(--x))",
            "clamp(1px, 2px)",
            "calc(1, 2)",
            "calc(2)",
            "calc(1px+2px)",
            "calc(1px -2px)",
            "calc(1px- 2px)",
            "calc(1e999px)",
            "calc(1e308px * 1e308)",
            "calc(1e308px + 1e308px)",
            "min(calc(1e308px * 1e308), 2px)",
        ] {
            assert!(evaluate(bad, &unit).is_err(), "{bad}");
        }
        let deep = format!("{}1px{}", "calc(".repeat(40), ")".repeat(40));
        assert!(evaluate(&deep, &unit).is_err(), "nesting is bounded");
    }

    #[test]
    fn functions_are_told_from_other_values() {
        assert!(is_function("calc(1px + 2px)"));
        assert!(is_function(" MIN(1px, 2px) "));
        assert!(!is_function("10px"));
        assert!(is_function("calc(1px"));
        assert!(is_function("calc(1px) extra"));
        assert!(!is_function("minimum(1px)"));
    }
}
