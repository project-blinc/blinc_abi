//! A parsed stylesheet as compact bytes, loaded without parsing: what
//! compile-time CSS produces in every SDK.
//!
//! The bytes are `BCSS`, a little-endian `u16` version, the atoms' text and
//! each atom's range in it, then each of the sheet's arenas in turn, as they
//! are. Counts, indexes, atoms and spans are LEB128; numbers are `f64`; an
//! optional atom is its index plus one, zero for none. Reading checks every
//! span and atom against the arena it names, and that a nested selector list
//! or feature comes before what holds it, so bytes from anywhere load safely.

use super::*;

pub const MAGIC: &[u8; 4] = b"BCSS";
/// The layout's version; a reader refuses any other.
pub const VERSION: u16 = 2;

/// `sheet` as compiled bytes.
pub fn encode(sheet: &Stylesheet) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(1024));
    w.0.extend_from_slice(MAGIC);
    w.0.extend_from_slice(&VERSION.to_le_bytes());
    let (text, spans) = sheet.atoms.parts();
    w.n(text.len() as u64);
    w.0.extend_from_slice(text.as_bytes());
    w.n(spans.len() as u64);
    for &(s, e) in spans {
        w.n(s as u64);
        w.n(e as u64);
    }
    w.list(&sheet.rules, |w, r| {
        w.span(r.selectors);
        w.span(r.declarations);
        match r.media {
            None => w.byte(0),
            Some(m) => {
                w.byte(1);
                w.span(m);
            }
        }
        w.n(r.order as u64);
        w.n(r.line as u64);
    });
    w.list(&sheet.selectors, |w, s| {
        w.span(s.compounds);
        w.span(s.combinators);
        w.byte(s.leading.map_or(0, |c| combinator_code(c) + 1));
        w.n(s.specificity as u64);
    });
    w.list(&sheet.compounds, |w, c| {
        w.opt(c.type_name);
        w.opt(c.id);
        w.span(c.classes);
        w.span(c.attributes);
        w.span(c.pseudos);
        w.opt(c.pseudo_element);
    });
    w.list(&sheet.combinators, |w, &c| w.byte(combinator_code(c)));
    w.list(&sheet.classes, |w, &a| w.atom(a));
    w.list(&sheet.attributes, |w, a| {
        w.atom(a.name);
        match a.test {
            None => w.byte(0),
            Some((op, v)) => {
                w.byte(op_code(op) + 1);
                w.atom(v);
            }
        }
    });
    w.list(&sheet.pseudos, |w, p| w.pseudo(p));
    w.list(&sheet.declarations, |w, d| {
        w.atom(d.name);
        w.atom(d.value);
        w.byte(d.important as u8);
        w.n(d.line as u64);
        w.n(d.column as u64);
    });
    w.list(&sheet.media_lists, |w, &s| w.span(s));
    w.list(&sheet.queries, |w, q| {
        w.byte(q.not as u8);
        w.span(q.features);
    });
    w.list(&sheet.features, |w, f| w.feature(f));
    w.list(&sheet.variables, |w, &(k, v)| {
        w.atom(k);
        w.atom(v);
    });
    w.list(&sheet.keyframes, |w, k| {
        w.atom(k.name);
        w.span(k.frames);
    });
    w.list(&sheet.frames, |w, f| {
        w.span(f.offsets);
        w.span(f.declarations);
    });
    w.list(&sheet.offsets, |w, &o| {
        w.0.extend_from_slice(&o.to_le_bytes())
    });
    w.list(&sheet.imports, |w, &a| w.atom(a));
    w.list(&sheet.diagnostics, |w, d| {
        w.byte((d.severity == Severity::Warning) as u8);
        w.str(&d.message);
        w.n(d.line as u64);
        w.n(d.column as u64);
        match &d.file {
            None => w.byte(0),
            Some(f) => {
                w.byte(1);
                w.str(f);
            }
        }
    });
    w.0
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
    let mut r = Reader { bytes, at: 6 };
    let text = r.str()?;
    let mut spans = Vec::new();
    for _ in 0..r.count()? {
        let s = r.u32()?;
        let e = r.u32()?;
        spans.push((s, e));
    }
    let atoms = Atoms::from_parts(text, spans).ok_or("an atom's range is outside its text")?;
    let mut sheet = Stylesheet {
        atoms,
        ..Default::default()
    };
    sheet.rules = r.list(|r| {
        let selectors = r.span()?;
        let declarations = r.span()?;
        let media = if r.byte()? == 0 {
            None
        } else {
            Some(r.span()?)
        };
        Ok(Rule {
            selectors,
            declarations,
            media,
            order: r.u32()?,
            line: r.u32()?,
        })
    })?;
    sheet.selectors = r.list(|r| {
        let compounds = r.span()?;
        let combinators = r.span()?;
        let leading = match r.byte()? {
            0 => None,
            c => Some(combinator(c - 1)?),
        };
        Ok(Selector {
            compounds,
            combinators,
            leading,
            specificity: r.u32()?,
        })
    })?;
    sheet.compounds = r.list(|r| {
        Ok(Compound {
            type_name: r.opt()?,
            id: r.opt()?,
            classes: r.span()?,
            attributes: r.span()?,
            pseudos: r.span()?,
            pseudo_element: r.opt()?,
        })
    })?;
    sheet.combinators = r.list(|r| combinator(r.byte()?))?;
    sheet.classes = r.list(|r| r.atom())?;
    sheet.attributes = r.list(|r| {
        let name = r.atom()?;
        let test = match r.byte()? {
            0 => None,
            op => Some((attribute_op(op - 1)?, r.atom()?)),
        };
        Ok(Attribute { name, test })
    })?;
    sheet.pseudos = r.list(|r| r.pseudo())?;
    sheet.declarations = r.list(|r| {
        Ok(Declaration {
            name: r.atom()?,
            value: r.atom()?,
            important: r.byte()? != 0,
            line: r.u32()?,
            column: r.u32()?,
        })
    })?;
    sheet.media_lists = r.list(|r| r.span())?;
    sheet.queries = r.list(|r| {
        Ok(MediaQuery {
            not: r.byte()? != 0,
            features: r.span()?,
        })
    })?;
    sheet.features = r.list(|r| r.feature())?;
    sheet.variables = r.list(|r| Ok((r.atom()?, r.atom()?)))?;
    sheet.keyframes = r.list(|r| {
        Ok(Keyframes {
            name: r.atom()?,
            frames: r.span()?,
        })
    })?;
    sheet.frames = r.list(|r| {
        Ok(Keyframe {
            offsets: r.span()?,
            declarations: r.span()?,
        })
    })?;
    sheet.offsets = r.list(|r| r.f())?;
    sheet.imports = r.list(|r| r.atom())?;
    sheet.diagnostics = r.list(|r| {
        let severity = if r.byte()? == 0 {
            Severity::Error
        } else {
            Severity::Warning
        };
        let message = r.str()?;
        let line = r.u32()?;
        let column = r.u32()?;
        let file = if r.byte()? == 0 { None } else { Some(r.str()?) };
        Ok(Diagnostic {
            severity,
            message,
            line,
            column,
            file,
        })
    })?;
    if r.at != bytes.len() {
        return Err("bytes after the sheet".into());
    }
    check(&sheet)?;
    Ok(sheet)
}

/// Every span within its arena and every atom within the table; nested lists
/// and features before what holds them, so walking a sheet always ends.
fn check(s: &Stylesheet) -> Result<(), String> {
    let within = |span: Span, len: usize| {
        (span.start as usize)
            .checked_add(span.len as usize)
            .is_some_and(|e| e <= len)
    };
    let atom = |a: Atom| (a.0 as usize) < s.atoms.len();
    let bad = |what: &str| Err(format!("{what} points outside the sheet"));
    for r in &s.rules {
        if !within(r.selectors, s.selectors.len()) || !within(r.declarations, s.declarations.len())
        {
            return bad("a rule");
        }
        if r.media.is_some_and(|m| !within(m, s.media_lists.len())) {
            return bad("a rule's media");
        }
    }
    for (i, sel) in s.selectors.iter().enumerate() {
        if !within(sel.compounds, s.compounds.len())
            || sel.compounds.is_empty()
            || !within(sel.combinators, s.combinators.len())
            || sel.combinators.len + 1 != sel.compounds.len
        {
            return bad("a selector");
        }
        for c in &s.compounds[sel.compounds.range()] {
            if !within(c.classes, s.classes.len())
                || !within(c.attributes, s.attributes.len())
                || !within(c.pseudos, s.pseudos.len())
            {
                return bad("a compound");
            }
            if [c.type_name, c.id, c.pseudo_element]
                .into_iter()
                .flatten()
                .any(|a| !atom(a))
            {
                return bad("a compound's name");
            }
            for p in &s.pseudos[c.pseudos.range()] {
                match *p {
                    Pseudo::Not(l) | Pseudo::Is(l) | Pseudo::Where(l) | Pseudo::Has(l) => {
                        if !within(l, i) || l.is_empty() {
                            return bad("a nested selector list");
                        }
                    }
                    Pseudo::State(a) if !atom(a) => return bad("a state"),
                    _ => {}
                }
            }
        }
    }
    if s.classes.iter().any(|&a| !atom(a))
        || s.attributes
            .iter()
            .any(|a| !atom(a.name) || a.test.is_some_and(|(_, v)| !atom(v)))
        || s.declarations
            .iter()
            .any(|d| !atom(d.name) || !atom(d.value))
        || s.variables.iter().any(|&(k, v)| !atom(k) || !atom(v))
        || s.imports.iter().any(|&a| !atom(a))
        || s.keyframes
            .iter()
            .any(|k| !atom(k.name) || !within(k.frames, s.frames.len()))
    {
        return bad("an atom or a span");
    }
    if s.media_lists.iter().any(|&l| !within(l, s.queries.len()))
        || s.queries
            .iter()
            .any(|q| !within(q.features, s.features.len()))
    {
        return bad("a media query");
    }
    for (i, f) in s.features.iter().enumerate() {
        if let MediaFeature::Both(a, b) = *f
            && (a as usize >= i || b as usize >= i)
        {
            return bad("a media range");
        }
    }
    if s.frames.iter().any(|f| {
        !within(f.offsets, s.offsets.len()) || !within(f.declarations, s.declarations.len())
    }) {
        return bad("a keyframe");
    }
    Ok(())
}

struct Writer(Vec<u8>);

impl Writer {
    fn n(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }

    fn byte(&mut self, b: u8) {
        self.0.push(b);
    }

    fn atom(&mut self, a: Atom) {
        self.n(a.0 as u64);
    }

    fn opt(&mut self, a: Option<Atom>) {
        self.n(a.map_or(0, |a| a.0 as u64 + 1));
    }

    fn span(&mut self, s: Span) {
        self.n(s.start as u64);
        self.n(s.len as u64);
    }

    fn str(&mut self, s: &str) {
        self.n(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes());
    }

    fn list<T>(&mut self, items: &[T], mut f: impl FnMut(&mut Self, &T)) {
        self.n(items.len() as u64);
        for x in items {
            f(self, x);
        }
    }

    fn pseudo(&mut self, p: &Pseudo) {
        let nth = |w: &mut Self, n: Nth| {
            w.n(zigzag(n.a));
            w.n(zigzag(n.b));
        };
        match *p {
            Pseudo::State(a) => {
                self.byte(0);
                self.atom(a);
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
                self.span(s);
            }
            Pseudo::Is(s) => {
                self.byte(14);
                self.span(s);
            }
            Pseudo::Where(s) => {
                self.byte(15);
                self.span(s);
            }
            Pseudo::Has(s) => {
                self.byte(16);
                self.span(s);
            }
        }
    }

    fn feature(&mut self, f: &MediaFeature) {
        match *f {
            MediaFeature::Width(op, v)
            | MediaFeature::Height(op, v)
            | MediaFeature::AspectRatio(op, v) => {
                self.byte(match f {
                    MediaFeature::Width(..) => 0,
                    MediaFeature::Height(..) => 1,
                    _ => 2,
                });
                self.byte(compare_code(op));
                self.0.extend_from_slice(&v.to_le_bytes());
            }
            MediaFeature::Orientation(b) => {
                self.byte(3);
                self.byte(b as u8);
            }
            MediaFeature::ColorScheme(b) => {
                self.byte(4);
                self.byte(b as u8);
            }
            MediaFeature::Fixed(b) => {
                self.byte(5);
                self.byte(b as u8);
            }
            MediaFeature::Both(a, b) => {
                self.byte(6);
                self.n(a as u64);
                self.n(b as u64);
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

fn combinator(code: u8) -> Result<Combinator, String> {
    Ok(match code {
        0 => Combinator::Descendant,
        1 => Combinator::Child,
        2 => Combinator::NextSibling,
        3 => Combinator::LaterSibling,
        _ => return Err("a combinator is out of range".into()),
    })
}

fn op_code(op: AttributeOp) -> u8 {
    match op {
        AttributeOp::Equals => 0,
        AttributeOp::Includes => 1,
        AttributeOp::DashMatch => 2,
        AttributeOp::Prefix => 3,
        AttributeOp::Suffix => 4,
        AttributeOp::Substring => 5,
    }
}

fn attribute_op(code: u8) -> Result<AttributeOp, String> {
    Ok(match code {
        0 => AttributeOp::Equals,
        1 => AttributeOp::Includes,
        2 => AttributeOp::DashMatch,
        3 => AttributeOp::Prefix,
        4 => AttributeOp::Suffix,
        5 => AttributeOp::Substring,
        _ => return Err("an attribute test is out of range".into()),
    })
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

    /// A count, bounded by the bytes left, so a bad one cannot ask for a huge allocation.
    fn count(&mut self) -> Got<usize> {
        let n = self.n()? as usize;
        if n > self.bytes.len() - self.at {
            return Err("a count is larger than the bytes left".into());
        }
        Ok(n)
    }

    fn u32(&mut self) -> Got<u32> {
        u32::try_from(self.n()?).map_err(|_| "a number is out of range".into())
    }

    fn f(&mut self) -> Got<f64> {
        let b = self
            .bytes
            .get(self.at..self.at + 8)
            .ok_or("the bytes end early")?;
        self.at += 8;
        Ok(f64::from_le_bytes(b.try_into().unwrap()))
    }

    fn atom(&mut self) -> Got<Atom> {
        Ok(Atom(self.u32()?))
    }

    fn opt(&mut self) -> Got<Option<Atom>> {
        Ok(match self.u32()? {
            0 => None,
            i => Some(Atom(i - 1)),
        })
    }

    fn span(&mut self) -> Got<Span> {
        Ok(Span {
            start: self.u32()?,
            len: self.u32()?,
        })
    }

    fn str(&mut self) -> Got<String> {
        let len = self.count()?;
        let s = std::str::from_utf8(&self.bytes[self.at..self.at + len])
            .map_err(|_| "a string is not UTF-8")?;
        self.at += len;
        Ok(s.to_string())
    }

    fn list<T>(&mut self, mut f: impl FnMut(&mut Self) -> Got<T>) -> Got<Vec<T>> {
        let n = self.count()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(f(self)?);
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
            0 => Pseudo::State(self.atom()?),
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
            13 => Pseudo::Not(self.span()?),
            14 => Pseudo::Is(self.span()?),
            15 => Pseudo::Where(self.span()?),
            16 => Pseudo::Has(self.span()?),
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
            6 => MediaFeature::Both(self.u32()?, self.u32()?),
            _ => return Err("a media feature is out of range".into()),
        })
    }
}
