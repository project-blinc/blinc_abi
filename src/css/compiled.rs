//! A parsed stylesheet as compact bytes, loaded without parsing: what
//! compile-time CSS produces in every SDK.
//!
//! The bytes are `BCSS`, a little-endian `u16` version, then a table of the
//! sheet's distinct strings (each once), then the sheet, its strings as
//! indexes into the table. Counts and indexes are LEB128; numbers are `f64`.
//! An optional string is its index plus one, zero for none.

use super::*;
use std::collections::HashMap;

pub const MAGIC: &[u8; 4] = b"BCSS";
/// The layout's version; a reader refuses any other.
pub const VERSION: u16 = 1;

/// `sheet` as compiled bytes.
pub fn encode(sheet: &Stylesheet) -> Vec<u8> {
    let mut w = Writer::default();
    w.sheet(sheet);
    let mut out = Vec::with_capacity(w.body.len() + w.table_len + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    leb(&mut out, w.strings.len() as u64);
    for s in &w.strings {
        leb(&mut out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }
    out.extend_from_slice(&w.body);
    out
}

/// The sheet in compiled bytes; an error for bytes that are not one, or of another version.
pub fn decode(bytes: &[u8]) -> Result<Stylesheet, String> {
    if bytes.len() < 6 || &bytes[..4] != MAGIC {
        return Err("not a compiled stylesheet".into());
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(format!(
            "compiled stylesheet version {version}; this reads {VERSION}"
        ));
    }
    let mut r = Reader {
        bytes,
        at: 6,
        strings: Vec::new(),
    };
    let n = r.count()?;
    for _ in 0..n {
        let len = r.count()?;
        let end =
            r.at.checked_add(len)
                .filter(|&e| e <= bytes.len())
                .ok_or("a string runs past the end")?;
        let s = std::str::from_utf8(&bytes[r.at..end]).map_err(|_| "a string is not UTF-8")?;
        r.strings.push(s.to_string());
        r.at = end;
    }
    let sheet = r.sheet()?;
    if r.at != bytes.len() {
        return Err("bytes after the sheet".into());
    }
    Ok(sheet)
}

fn leb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

#[derive(Default)]
struct Writer {
    strings: Vec<String>,
    index: HashMap<String, u64>,
    table_len: usize,
    body: Vec<u8>,
}

impl Writer {
    fn n(&mut self, v: u64) {
        leb(&mut self.body, v);
    }

    fn byte(&mut self, b: u8) {
        self.body.push(b);
    }

    fn f(&mut self, v: f64) {
        self.body.extend_from_slice(&v.to_le_bytes());
    }

    fn str_index(&mut self, s: &str) -> u64 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.strings.len() as u64;
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), i);
        self.table_len += s.len() + 2;
        i
    }

    fn s(&mut self, s: &str) {
        let i = self.str_index(s);
        self.n(i);
    }

    fn opt(&mut self, s: Option<&str>) {
        match s {
            None => self.n(0),
            Some(s) => {
                let i = self.str_index(s);
                self.n(i + 1);
            }
        }
    }

    fn sheet(&mut self, sheet: &Stylesheet) {
        self.n(sheet.rules.len() as u64);
        for r in &sheet.rules {
            self.selectors(&r.selectors);
            self.declarations(&r.declarations);
            match &r.media {
                None => self.byte(0),
                Some(lists) => {
                    self.byte(1);
                    self.n(lists.len() as u64);
                    for l in lists {
                        self.n(l.len() as u64);
                        for q in l {
                            self.byte(q.not as u8);
                            self.n(q.features.len() as u64);
                            for f in &q.features {
                                self.feature(f);
                            }
                        }
                    }
                }
            }
            self.n(r.order as u64);
            self.n(r.line as u64);
        }
        self.n(sheet.variables.len() as u64);
        for (k, v) in &sheet.variables {
            self.s(k);
            self.s(v);
        }
        self.n(sheet.keyframes.len() as u64);
        for k in &sheet.keyframes {
            self.s(&k.name);
            self.n(k.frames.len() as u64);
            for f in &k.frames {
                self.n(f.offsets.len() as u64);
                for &o in &f.offsets {
                    self.f(o);
                }
                self.declarations(&f.declarations);
            }
        }
        self.n(sheet.imports.len() as u64);
        for i in &sheet.imports {
            self.s(i);
        }
        self.n(sheet.diagnostics.len() as u64);
        for d in &sheet.diagnostics {
            self.byte((d.severity == Severity::Warning) as u8);
            self.s(&d.message);
            self.n(d.line as u64);
            self.n(d.column as u64);
            self.opt(d.file.as_deref());
        }
    }

    fn declarations(&mut self, list: &[Declaration]) {
        self.n(list.len() as u64);
        for d in list {
            self.s(&d.name);
            self.s(&d.value);
            self.byte(d.important as u8);
            self.n(d.line as u64);
            self.n(d.column as u64);
        }
    }

    fn selectors(&mut self, list: &[Selector]) {
        self.n(list.len() as u64);
        for s in list {
            self.byte(s.leading.map_or(0, |c| combinator_code(c) + 1));
            self.n(s.compounds.len() as u64);
            for c in &s.compounds {
                self.opt(c.type_name.as_deref());
                self.opt(c.id.as_deref());
                self.n(c.classes.len() as u64);
                for x in &c.classes {
                    self.s(x);
                }
                self.n(c.attributes.len() as u64);
                for a in &c.attributes {
                    self.s(&a.name);
                    self.opt(a.op.as_deref());
                    self.opt(a.value.as_deref());
                }
                self.n(c.pseudos.len() as u64);
                for p in &c.pseudos {
                    self.pseudo(p);
                }
                self.opt(c.pseudo_element.as_deref());
            }
            for &k in &s.combinators {
                self.byte(combinator_code(k));
            }
        }
    }

    fn pseudo(&mut self, p: &Pseudo) {
        let nth = |w: &mut Self, n: &Nth| {
            w.n(zigzag(n.a));
            w.n(zigzag(n.b));
        };
        match p {
            Pseudo::State(s) => {
                self.byte(0);
                self.s(s);
            }
            Pseudo::Root => self.byte(1),
            Pseudo::Empty => self.byte(2),
            Pseudo::FirstChild => self.byte(3),
            Pseudo::LastChild => self.byte(4),
            Pseudo::OnlyChild => self.byte(5),
            Pseudo::NthChild(n) => {
                self.byte(6);
                nth(self, n);
            }
            Pseudo::NthLastChild(n) => {
                self.byte(7);
                nth(self, n);
            }
            Pseudo::FirstOfType => self.byte(8),
            Pseudo::LastOfType => self.byte(9),
            Pseudo::OnlyOfType => self.byte(10),
            Pseudo::NthOfType(n) => {
                self.byte(11);
                nth(self, n);
            }
            Pseudo::NthLastOfType(n) => {
                self.byte(12);
                nth(self, n);
            }
            Pseudo::Not(s) => {
                self.byte(13);
                self.selectors(s);
            }
            Pseudo::Is(s) => {
                self.byte(14);
                self.selectors(s);
            }
            Pseudo::Where(s) => {
                self.byte(15);
                self.selectors(s);
            }
            Pseudo::Has(s) => {
                self.byte(16);
                self.selectors(s);
            }
        }
    }

    fn feature(&mut self, f: &MediaFeature) {
        match f {
            MediaFeature::Width(op, v)
            | MediaFeature::Height(op, v)
            | MediaFeature::AspectRatio(op, v) => {
                self.byte(match f {
                    MediaFeature::Width(..) => 0,
                    MediaFeature::Height(..) => 1,
                    _ => 2,
                });
                self.byte(compare_code(*op));
                self.f(*v);
            }
            MediaFeature::Orientation(b) => {
                self.byte(3);
                self.byte(*b as u8);
            }
            MediaFeature::ColorScheme(b) => {
                self.byte(4);
                self.byte(*b as u8);
            }
            MediaFeature::Fixed(b) => {
                self.byte(5);
                self.byte(*b as u8);
            }
            MediaFeature::Both(a, b) => {
                self.byte(6);
                self.feature(a);
                self.feature(b);
            }
        }
    }
}

fn zigzag(v: i32) -> u64 {
    ((v << 1) ^ (v >> 31)) as u32 as u64
}

fn unzigzag(v: u64) -> i32 {
    let v = v as u32;
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

fn combinator_code(c: Combinator) -> u8 {
    match c {
        Combinator::Descendant => 0,
        Combinator::Child => 1,
        Combinator::NextSibling => 2,
        Combinator::LaterSibling => 3,
    }
}

fn compare_code(c: Compare) -> u8 {
    match c {
        Compare::Eq => 0,
        Compare::Lt => 1,
        Compare::Le => 2,
        Compare::Gt => 3,
        Compare::Ge => 4,
    }
}

struct Reader<'b> {
    bytes: &'b [u8],
    at: usize,
    strings: Vec<String>,
}

type Got<T> = Result<T, String>;

impl Reader<'_> {
    fn byte(&mut self) -> Got<u8> {
        let b = *self.bytes.get(self.at).ok_or("the bytes end early")?;
        self.at += 1;
        Ok(b)
    }

    fn n(&mut self) -> Got<u64> {
        let (mut v, mut shift) = (0u64, 0);
        loop {
            let b = self.byte()?;
            if shift > 63 {
                return Err("a number is too long".into());
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    /// A count, bounded by the bytes left so a bad one cannot ask for huge allocations.
    fn count(&mut self) -> Got<usize> {
        let n = self.n()? as usize;
        if n > self.bytes.len() {
            return Err("a count is larger than the bytes".into());
        }
        Ok(n)
    }

    fn u32(&mut self) -> Got<u32> {
        u32::try_from(self.n()?).map_err(|_| "a number is out of range".into())
    }

    fn f(&mut self) -> Got<f64> {
        let end = self.at + 8;
        let b = self.bytes.get(self.at..end).ok_or("the bytes end early")?;
        self.at = end;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }

    fn s(&mut self) -> Got<String> {
        let i = self.n()? as usize;
        self.strings
            .get(i)
            .cloned()
            .ok_or_else(|| "a string index is out of range".into())
    }

    fn opt(&mut self) -> Got<Option<String>> {
        match self.n()? as usize {
            0 => Ok(None),
            i => self
                .strings
                .get(i - 1)
                .cloned()
                .map(Some)
                .ok_or_else(|| "a string index is out of range".into()),
        }
    }

    fn sheet(&mut self) -> Got<Stylesheet> {
        let mut sheet = Stylesheet::default();
        for _ in 0..self.count()? {
            let selectors = self.selectors()?;
            let declarations = self.declarations()?;
            let media = match self.byte()? {
                0 => None,
                _ => {
                    let mut lists = Vec::new();
                    for _ in 0..self.count()? {
                        let mut list = Vec::new();
                        for _ in 0..self.count()? {
                            let not = self.byte()? != 0;
                            let mut features = Vec::new();
                            for _ in 0..self.count()? {
                                features.push(self.feature()?);
                            }
                            list.push(MediaQuery { not, features });
                        }
                        lists.push(list);
                    }
                    Some(lists)
                }
            };
            let order = self.u32()?;
            let line = self.u32()?;
            sheet.rules.push(StyleRule {
                selectors,
                declarations,
                media,
                order,
                line,
            });
        }
        for _ in 0..self.count()? {
            let k = self.s()?;
            let v = self.s()?;
            sheet.variables.push((k, v));
        }
        for _ in 0..self.count()? {
            let name = self.s()?;
            let mut frames = Vec::new();
            for _ in 0..self.count()? {
                let mut offsets = Vec::new();
                for _ in 0..self.count()? {
                    offsets.push(self.f()?);
                }
                frames.push(Keyframe {
                    offsets,
                    declarations: self.declarations()?,
                });
            }
            sheet.keyframes.push(Keyframes { name, frames });
        }
        for _ in 0..self.count()? {
            let i = self.s()?;
            sheet.imports.push(i);
        }
        for _ in 0..self.count()? {
            let severity = if self.byte()? == 0 {
                Severity::Error
            } else {
                Severity::Warning
            };
            let message = self.s()?;
            let line = self.u32()?;
            let column = self.u32()?;
            let file = self.opt()?;
            sheet.diagnostics.push(Diagnostic {
                severity,
                message,
                line,
                column,
                file,
            });
        }
        Ok(sheet)
    }

    fn declarations(&mut self) -> Got<Vec<Declaration>> {
        let mut out = Vec::new();
        for _ in 0..self.count()? {
            let name = self.s()?;
            let value = self.s()?;
            let important = self.byte()? != 0;
            let line = self.u32()?;
            let column = self.u32()?;
            out.push(Declaration {
                name,
                value,
                important,
                line,
                column,
            });
        }
        Ok(out)
    }

    fn combinator(code: u8) -> Got<Combinator> {
        Ok(match code {
            0 => Combinator::Descendant,
            1 => Combinator::Child,
            2 => Combinator::NextSibling,
            3 => Combinator::LaterSibling,
            _ => return Err("a combinator is out of range".into()),
        })
    }

    fn selectors(&mut self) -> Got<Vec<Selector>> {
        let mut out = Vec::new();
        for _ in 0..self.count()? {
            let leading = match self.byte()? {
                0 => None,
                c => Some(Self::combinator(c - 1)?),
            };
            let mut compounds = Vec::new();
            for _ in 0..self.count()? {
                let type_name = self.opt()?;
                let id = self.opt()?;
                let mut classes = Vec::new();
                for _ in 0..self.count()? {
                    classes.push(self.s()?);
                }
                let mut attributes = Vec::new();
                for _ in 0..self.count()? {
                    let name = self.s()?;
                    let op = self.opt()?;
                    let value = self.opt()?;
                    attributes.push(Attribute { name, op, value });
                }
                let mut pseudos = Vec::new();
                for _ in 0..self.count()? {
                    pseudos.push(self.pseudo()?);
                }
                let pseudo_element = self.opt()?;
                compounds.push(Compound {
                    type_name,
                    id,
                    classes,
                    attributes,
                    pseudos,
                    pseudo_element,
                });
            }
            if compounds.is_empty() {
                return Err("a selector has no compounds".into());
            }
            let mut combinators = Vec::new();
            for _ in 1..compounds.len() {
                let c = self.byte()?;
                combinators.push(Self::combinator(c)?);
            }
            out.push(Selector {
                compounds,
                combinators,
                leading,
            });
        }
        Ok(out)
    }

    fn nth(&mut self) -> Got<Nth> {
        let a = unzigzag(self.n()?);
        let b = unzigzag(self.n()?);
        Ok(Nth { a, b })
    }

    fn pseudo(&mut self) -> Got<Pseudo> {
        Ok(match self.byte()? {
            0 => Pseudo::State(self.s()?),
            1 => Pseudo::Root,
            2 => Pseudo::Empty,
            3 => Pseudo::FirstChild,
            4 => Pseudo::LastChild,
            5 => Pseudo::OnlyChild,
            6 => Pseudo::NthChild(self.nth()?),
            7 => Pseudo::NthLastChild(self.nth()?),
            8 => Pseudo::FirstOfType,
            9 => Pseudo::LastOfType,
            10 => Pseudo::OnlyOfType,
            11 => Pseudo::NthOfType(self.nth()?),
            12 => Pseudo::NthLastOfType(self.nth()?),
            13 => Pseudo::Not(self.selectors()?),
            14 => Pseudo::Is(self.selectors()?),
            15 => Pseudo::Where(self.selectors()?),
            16 => Pseudo::Has(self.selectors()?),
            _ => return Err("a pseudo-class is out of range".into()),
        })
    }

    fn compare(&mut self) -> Got<Compare> {
        Ok(match self.byte()? {
            0 => Compare::Eq,
            1 => Compare::Lt,
            2 => Compare::Le,
            3 => Compare::Gt,
            4 => Compare::Ge,
            _ => return Err("a comparison is out of range".into()),
        })
    }

    fn feature(&mut self) -> Got<MediaFeature> {
        Ok(match self.byte()? {
            0 => MediaFeature::Width(self.compare()?, self.f()?),
            1 => MediaFeature::Height(self.compare()?, self.f()?),
            2 => MediaFeature::AspectRatio(self.compare()?, self.f()?),
            3 => MediaFeature::Orientation(self.byte()? != 0),
            4 => MediaFeature::ColorScheme(self.byte()? != 0),
            5 => MediaFeature::Fixed(self.byte()? != 0),
            6 => MediaFeature::Both(Box::new(self.feature()?), Box::new(self.feature()?)),
            _ => return Err("a media feature is out of range".into()),
        })
    }
}
