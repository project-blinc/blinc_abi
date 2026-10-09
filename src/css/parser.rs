//! Reads CSS text into a [`Stylesheet`], by recursive descent over the
//! characters. A rule it cannot read is skipped to its closing brace and
//! reported; the rest of the sheet still parses.
//!
//! Beyond plain rules it reads CSS nesting (`&`, nested rules and nested
//! `@media`, flattened to plain selectors), `@media` queries kept on each
//! rule they hold, `@import`ed files read through the sheet's loader, and
//! Sass's `@mixin name($param: default) { … }` with `@include name(args);`,
//! a mixin's body read again where it is included with its arguments put in.

use super::media::{self, MediaQuery};
use super::tree::{Attribute, Compound, Declaration, Pseudo, Selector};
use super::{Combinator, Diagnostic, Nth, STATES, Severity, Span, Stylesheet, value};
use std::collections::HashMap;

/// Reads an `@import`ed file: the path as written, and the file importing it
/// (none for CSS text with no file). Gives the file's source and its name.
pub type Loader<'l> = dyn FnMut(&str, Option<&str>) -> Option<(String, String)> + 'l;

/// `source` parsed; `file` names it in diagnostics and is where an `@import`
/// is found from. `load` reads imported files.
pub fn parse(source: &str, file: Option<&str>, load: &mut Loader) -> Stylesheet {
    let mut cx = Cx {
        sheet: Stylesheet::default(),
        order: 0,
        mixins: HashMap::new(),
        importing: file.map(|f| vec![f.to_string()]).unwrap_or_default(),
        load,
    };
    Parser::new(source, file.map(str::to_string), false, None).rules(&mut cx, &Scope::top(), false);
    // A nested rule is pushed before its parent's, which waits for its block; source order again.
    cx.sheet.rules.sort_by_key(|r| r.order);
    cx.sheet
}

/// A selector list on its own, `.card > .title, #save`, laid out in a sheet
/// of its own, and the span of the sheet's selectors it is. An error naming
/// the text for one it cannot read.
pub fn parse_selectors(text: &str) -> Result<(Stylesheet, Span), String> {
    let list = SelectorReader::new(text, 0)
        .list(false)
        .map_err(|f| format!("bad selector \"{text}\": {}", f.message))?;
    let mut sheet = Stylesheet::default();
    let span = sheet.push_selectors(&list);
    Ok((sheet, span))
}

struct Failure {
    message: String,
    at: usize,
}

/// Where rules being read stand: the selectors of the rules they are nested
/// in, as text, and the `@media` lists around them.
#[derive(Clone)]
struct Scope {
    parents: Option<Vec<String>>,
    media: Option<Vec<Vec<MediaQuery>>>,
}

impl Scope {
    fn top() -> Self {
        Scope {
            parents: None,
            media: None,
        }
    }
}

struct Mixin {
    params: Vec<(String, Option<String>)>,
    body: String,
}

/// What a sheet and the files it imports share while they are read.
struct Cx<'c, 'l> {
    sheet: Stylesheet,
    order: u32,
    mixins: HashMap<String, Mixin>,
    importing: Vec<String>,
    load: &'c mut Loader<'l>,
}

#[derive(Clone)]
struct Anchor {
    line: u32,
    column: u32,
    file: Option<String>,
}

struct Parser {
    src: Vec<char>,
    file: Option<String>,
    line_starts: Vec<usize>,
    /// An imported file's diagnostics name it; the sheet's own do not.
    imported: bool,
    /// Where to report a mixin body's problems: the `@include` it was read for.
    anchor: Option<Anchor>,
    at: usize,
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c as u32 >= 0x80
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

impl Parser {
    fn new(src: &str, file: Option<String>, imported: bool, anchor: Option<Anchor>) -> Self {
        let src: Vec<char> = src.chars().collect();
        let mut line_starts = vec![0];
        for (i, &c) in src.iter().enumerate() {
            if c == '\n' {
                line_starts.push(i + 1);
            }
        }
        Parser {
            src,
            file,
            line_starts,
            imported,
            anchor,
            at: 0,
        }
    }

    // --- Rules ---

    /// Rules to the end, or with `closed` to the `}` closing a block, consuming it.
    fn rules(&mut self, cx: &mut Cx, scope: &Scope, closed: bool) {
        loop {
            self.skip_space(cx);
            if self.done() {
                if closed {
                    self.report(cx, Severity::Error, "a block is not closed with }", self.at);
                }
                return;
            }
            match self.peek() {
                '@' => self.at_rule(cx, scope, None),
                '}' => {
                    self.at += 1;
                    if closed {
                        return;
                    }
                    self.report(cx, Severity::Error, "a } with no rule open", self.at - 1);
                }
                _ => self.style_rule(cx, scope, None),
            }
        }
    }

    /// A style rule at `scope`. Its selectors are composed with the parents':
    /// `&` stands for each, a selector without one is a descendant of each, one
    /// starting with a combinator relates to each. Its own declarations come
    /// before the rules nested in it.
    fn style_rule(&mut self, cx: &mut Cx, scope: &Scope, _into: Option<&mut Vec<Declaration>>) {
        let start = self.at;
        let prelude = self.until(cx, &['{', ';', '}']);
        if self.done() || self.peek() != '{' {
            self.report(
                cx,
                Severity::Error,
                &format!("expected {{ after \"{}\"", prelude.trim()),
                start,
            );
            if !self.done() && self.peek() == ';' {
                self.at += 1;
            }
            return;
        }
        self.at += 1;
        let texts = compose(scope.parents.as_deref(), &prelude);
        let selectors = match SelectorReader::new(&texts.join(", "), start).list(false) {
            Ok(s) => s,
            Err(f) => {
                self.report(cx, Severity::Error, &f.message, f.at);
                self.skip_block();
                return;
            }
        };
        let inner = Scope {
            parents: Some(selectors.iter().map(|s| s.to_string()).collect()),
            media: scope.media.clone(),
        };
        let order = cx.order;
        cx.order += 1;
        let declarations = self.block(cx, &inner, true);
        self.emit(
            cx,
            selectors,
            declarations,
            scope.media.clone(),
            order,
            start,
        );
    }

    /// Lays a rule out in the sheet; `:root`'s custom properties outside `@media` become its variables.
    fn emit(
        &self,
        cx: &mut Cx,
        selectors: Vec<Selector>,
        declarations: Vec<Declaration>,
        media: Option<Vec<Vec<MediaQuery>>>,
        order: u32,
        start: usize,
    ) {
        let root_only = media.is_none()
            && selectors
                .iter()
                .all(|s| s.compounds.len() == 1 && is_root(&s.compounds[0]));
        if root_only {
            for d in &declarations {
                if let Some(name) = d.name.strip_prefix("--") {
                    cx.sheet.set_variable(name, &d.value);
                }
            }
        }
        let sheet = &mut cx.sheet;
        let selectors = sheet.push_selectors(&selectors);
        let declarations = sheet.push_declarations(&declarations);
        let media = media.map(|m| sheet.push_media(&m));
        sheet.rules.push(super::Rule {
            selectors,
            declarations,
            media,
            order,
            line: self.line_of(start),
        });
    }

    /// An at-rule at `scope`. Inside a style rule (`into` its declarations)
    /// it may be `@media`, whose declarations go to a rule of its own under
    /// the query, or `@include`, whose declarations join `into`.
    fn at_rule(&mut self, cx: &mut Cx, scope: &Scope, into: Option<&mut Vec<Declaration>>) {
        let start = self.at;
        self.at += 1;
        let name = self.ident().to_lowercase();
        match name.as_str() {
            "media" => {
                let prelude = self.until(cx, &['{', ';']).trim().to_string();
                if self.done() || self.peek() != '{' {
                    self.report(cx, Severity::Error, "expected { after @media", start);
                    if !self.done() {
                        self.at += 1;
                    }
                    return;
                }
                self.at += 1;
                let list = match media::parse(&prelude) {
                    Ok(l) => l,
                    Err(e) => {
                        self.report(cx, Severity::Error, &format!("@media: {e}"), start);
                        self.skip_block();
                        return;
                    }
                };
                let mut lists = scope.media.clone().unwrap_or_default();
                lists.push(list);
                let inner = Scope {
                    parents: scope.parents.clone(),
                    media: Some(lists.clone()),
                };
                if scope.parents.is_none() {
                    self.rules(cx, &inner, true);
                } else {
                    let order = cx.order;
                    cx.order += 1;
                    let declarations = self.block(cx, &inner, true);
                    let parents = parent_selectors(scope);
                    self.emit(cx, parents, declarations, Some(lists), order, start);
                }
            }
            "import" => {
                let prelude = self.until(cx, &[';', '{', '}']).trim().to_string();
                if !self.done() && self.peek() == ';' {
                    self.at += 1;
                }
                if scope.parents.is_some() {
                    self.report(
                        cx,
                        Severity::Error,
                        "@import goes at the top of a sheet, not inside a rule",
                        start,
                    );
                    return;
                }
                self.import_file(cx, &prelude, scope, start);
            }
            "mixin" => self.define_mixin(cx, start),
            "include" => {
                let prelude = self.until(cx, &[';', '{', '}']).trim().to_string();
                if !self.done() && self.peek() == ';' {
                    self.at += 1;
                }
                match into {
                    None => self.report(cx, Severity::Error, "@include goes inside a rule", start),
                    Some(into) => self.include(cx, &prelude, scope, into, start),
                }
            }
            "keyframes" | "-webkit-keyframes" => {
                if scope.parents.is_some() {
                    self.report(
                        cx,
                        Severity::Error,
                        "@keyframes goes at the top of a sheet",
                        start,
                    );
                    self.skip_statement(cx);
                    return;
                }
                self.keyframes(cx, start);
            }
            _ => {
                self.report(
                    cx,
                    Severity::Warning,
                    &format!("@{name} is not supported; skipped"),
                    start,
                );
                self.skip_statement(cx);
            }
        }
    }

    /// Skips an at-rule's prelude and its block or `;`.
    fn skip_statement(&mut self, cx: &mut Cx) {
        self.until(cx, &['{', ';']);
        if !self.done() && self.peek() == '{' {
            self.at += 1;
            self.skip_block();
        } else if !self.done() {
            self.at += 1;
        }
    }

    /// `@import "a.css"` or `url(a.css)`, with an optional media list: its rules here, under that media.
    fn import_file(&mut self, cx: &mut Cx, prelude: &str, scope: &Scope, start: usize) {
        let Some((path, rest)) = import_target(prelude) else {
            self.report(
                cx,
                Severity::Error,
                &format!("@import takes a file, \"a.css\" or url(a.css), not \"{prelude}\""),
                start,
            );
            return;
        };
        let mut lists = scope.media.clone();
        if !rest.is_empty() {
            match media::parse(&rest) {
                Ok(list) => lists.get_or_insert_with(Vec::new).push(list),
                Err(e) => {
                    self.report(cx, Severity::Error, &format!("@import: {e}"), start);
                    return;
                }
            }
        }
        let Some((source, loaded)) = (cx.load)(&path, self.file.as_deref()) else {
            self.report(
                cx,
                Severity::Error,
                &format!("@import: no file {path}"),
                start,
            );
            return;
        };
        if cx.importing.contains(&loaded) {
            let chain = cx.importing.join(" -> ");
            self.report(
                cx,
                Severity::Error,
                &format!("@import: {path} imports itself, through {chain}"),
                start,
            );
            return;
        }
        let atom = cx.sheet.atoms.intern(&loaded);
        if !cx.sheet.imports.contains(&atom) {
            cx.sheet.imports.push(atom);
        }
        cx.importing.push(loaded.clone());
        Parser::new(&source, Some(loaded), true, None).rules(
            cx,
            &Scope {
                parents: None,
                media: lists,
            },
            false,
        );
        cx.importing.pop();
    }

    /// `@mixin name($a, $b: default) { … }`: kept as text, read where it is included.
    fn define_mixin(&mut self, cx: &mut Cx, start: usize) {
        self.skip_space(cx);
        let name = self.ident();
        let mut params = Vec::new();
        self.skip_space(cx);
        if !self.done() && self.peek() == '(' {
            self.at += 1;
            let inner = self.until(cx, &[')']);
            if !self.done() {
                self.at += 1;
            }
            for p in value::split(&inner, ',') {
                if p.is_empty() {
                    continue;
                }
                let colon = p.find(':');
                let pname = colon.map_or(p.as_str(), |c| &p[..c]).trim();
                if !pname.starts_with('$') {
                    self.report(
                        cx,
                        Severity::Error,
                        &format!("@mixin {name}: a parameter is $name, not \"{pname}\""),
                        start,
                    );
                    self.skip_statement(cx);
                    return;
                }
                params.push((
                    pname[1..].to_string(),
                    colon.map(|c| p[c + 1..].trim().to_string()),
                ));
            }
        }
        self.skip_space(cx);
        if name.is_empty() || self.done() || self.peek() != '{' {
            self.report(
                cx,
                Severity::Error,
                "expected a name and { after @mixin",
                start,
            );
            self.skip_statement(cx);
            return;
        }
        self.at += 1;
        let body_start = self.at;
        self.skip_block();
        let body = self.text(body_start, self.at - 1);
        cx.mixins.insert(name, Mixin { params, body });
    }

    /// `@include name(args)`: the mixin's body, its parameters replaced by the arguments, read here.
    fn include(
        &mut self,
        cx: &mut Cx,
        prelude: &str,
        scope: &Scope,
        into: &mut Vec<Declaration>,
        start: usize,
    ) {
        let call = value::call(prelude);
        let name = call
            .as_ref()
            .map_or(prelude.to_string(), |(n, _)| n.clone());
        let Some(mixin) = cx.mixins.get(&name) else {
            self.report(
                cx,
                Severity::Error,
                &format!("@include: no mixin {name} (a mixin is defined before it is included)"),
                start,
            );
            return;
        };
        let args: Vec<String> = call
            .map(|(_, a)| {
                value::split(&a, ',')
                    .into_iter()
                    .filter(|a| !a.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut values: HashMap<String, String> = HashMap::new();
        let mut positional = 0;
        let mut error = None;
        for a in &args {
            if let Some((n, v)) = named_argument(a) {
                values.insert(n, v);
            } else if positional < mixin.params.len() {
                values.insert(mixin.params[positional].0.clone(), a.clone());
                positional += 1;
            } else {
                error = Some(format!(
                    "@include {name}: more arguments than its {} parameters",
                    mixin.params.len()
                ));
                break;
            }
        }
        if error.is_none() {
            for (p, default) in &mixin.params {
                if !values.contains_key(p) {
                    match default {
                        None => {
                            error = Some(format!("@include {name}: no value for ${p}"));
                            break;
                        }
                        Some(d) => {
                            values.insert(p.clone(), d.clone());
                        }
                    }
                }
            }
        }
        if let Some(e) = error {
            self.report(cx, Severity::Error, &e, start);
            return;
        }
        // Longer names first, so $padding is not read as $pad followed by "ding".
        let mut names: Vec<&String> = values.keys().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let mut body = mixin.body.clone();
        for n in names {
            body = body.replace(&format!("${n}"), &values[n]);
        }
        let here = Anchor {
            line: self.line_of(start),
            column: self.column_of(start),
            file: if self.imported {
                self.file.clone()
            } else {
                None
            },
        };
        let anchor = self.anchor.clone().unwrap_or(here);
        let mut reader = Parser::new(&body, self.file.clone(), self.imported, Some(anchor));
        let declarations = reader.block(cx, scope, false);
        into.extend(declarations);
    }

    fn keyframes(&mut self, cx: &mut Cx, start: usize) {
        self.skip_space(cx);
        let name = if !self.done() && (self.peek() == '"' || self.peek() == '\'') {
            self.quoted(cx)
        } else {
            self.ident()
        };
        self.skip_space(cx);
        if name.is_empty() || self.done() || self.peek() != '{' {
            self.report(
                cx,
                Severity::Error,
                "expected a name and { after @keyframes",
                start,
            );
            self.until(cx, &['{']);
            if !self.done() {
                self.at += 1;
                self.skip_block();
            }
            return;
        }
        self.at += 1;
        let mut frames = Vec::new();
        loop {
            self.skip_space(cx);
            if self.done() {
                self.report(
                    cx,
                    Severity::Error,
                    &format!("@keyframes {name} is not closed"),
                    start,
                );
                break;
            }
            if self.peek() == '}' {
                self.at += 1;
                break;
            }
            let step_at = self.at;
            let prelude = self.until(cx, &['{', '}']);
            if self.done() || self.peek() != '{' {
                self.report(
                    cx,
                    Severity::Error,
                    "expected { after a keyframe offset",
                    step_at,
                );
                continue;
            }
            self.at += 1;
            let mut offsets = Vec::new();
            let mut bad = false;
            for part in prelude.split(',') {
                let p = part.trim().to_lowercase();
                let v = match p.as_str() {
                    "from" => 0.0,
                    "to" => 1.0,
                    _ if p.ends_with('%') => parse_float_prefix(&p[..p.len() - 1]) / 100.0,
                    _ => f64::NAN,
                };
                if v.is_nan() || !(0.0..=1.0).contains(&v) {
                    self.report(
                        cx,
                        Severity::Error,
                        &format!(
                            "a keyframe offset is from, to or a percentage 0% to 100%, not \"{p}\""
                        ),
                        step_at,
                    );
                    bad = true;
                }
                offsets.push(v);
            }
            let declarations = self.block(cx, &Scope::top(), true);
            if !bad {
                frames.push((offsets, declarations));
            }
        }
        cx.sheet.set_keyframes(&name, &frames);
    }

    /// A block's declarations, to its `}` when `closed` (consumed), else to
    /// the end; rules and `@media` nested in it are read as rules of their
    /// own at `scope`, and `@include`s join their declarations to its.
    fn block(&mut self, cx: &mut Cx, scope: &Scope, closed: bool) -> Vec<Declaration> {
        let mut out = Vec::new();
        loop {
            self.skip_space(cx);
            if self.done() {
                if closed {
                    self.report(cx, Severity::Error, "a block is not closed with }", self.at);
                }
                return out;
            }
            match self.peek() {
                '}' => {
                    self.at += 1;
                    if closed {
                        return out;
                    }
                    self.report(cx, Severity::Error, "a } with no block open", self.at - 1);
                    continue;
                }
                ';' => {
                    self.at += 1;
                    continue;
                }
                '@' => {
                    self.at_rule(cx, scope, Some(&mut out));
                    continue;
                }
                _ => {}
            }
            // A rule nested here reaches { before ; or }; a declaration does not.
            let start = self.at;
            self.until(cx, &[';', '{', '}']);
            let nested = !self.done() && self.peek() == '{';
            self.at = start;
            if nested {
                if scope.parents.is_none() {
                    self.report(cx, Severity::Error, "a rule cannot nest here", start);
                    self.until(cx, &['{']);
                    self.at += 1;
                    self.skip_block();
                } else {
                    self.style_rule(cx, scope, Some(&mut out));
                }
                continue;
            }
            let name = self.until(cx, &[':', ';', '}']).trim().to_string();
            if self.done() || self.peek() != ':' {
                self.report(
                    cx,
                    Severity::Error,
                    &format!("expected : after \"{name}\""),
                    start,
                );
                continue;
            }
            self.at += 1;
            let value_at = self.at;
            let mut value = self.until(cx, &[';', '}']).trim().to_string();
            let mut important = false;
            if let Some(left) = strip_important(&value) {
                important = true;
                value = left.trim().to_string();
            }
            if name.is_empty() || !is_property_name(&name) {
                self.report(
                    cx,
                    Severity::Error,
                    &format!("\"{name}\" is not a property name"),
                    start,
                );
                continue;
            }
            let custom = name.starts_with("--");
            if value.is_empty() && !custom {
                self.report(
                    cx,
                    Severity::Error,
                    &format!("{name} has no value"),
                    value_at,
                );
                continue;
            }
            let (line, column) = match &self.anchor {
                Some(a) => (a.line, a.column),
                None => (self.line_of(start), self.column_of(start)),
            };
            out.push(Declaration {
                name: if custom { name } else { name.to_lowercase() },
                value,
                important,
                line,
                column,
            });
        }
    }

    // --- Characters ---

    /// The text up to the first of `stops` outside strings, parentheses and
    /// brackets, or to the end; the stop is not consumed.
    fn until(&mut self, cx: &mut Cx, stops: &[char]) -> String {
        let start = self.at;
        let mut depth = 0;
        while !self.done() {
            let c = self.peek();
            if c == '"' || c == '\'' {
                self.quoted(cx);
                continue;
            }
            if c == '/' && self.code(self.at + 1) == Some('*') {
                self.skip_space(cx);
                continue;
            }
            if c == '\\' {
                self.at += 2;
                continue;
            }
            if depth == 0 && stops.contains(&c) {
                break;
            }
            if c == '(' || c == '[' {
                depth += 1;
            } else if (c == ')' || c == ']') && depth > 0 {
                depth -= 1;
            }
            self.at += 1;
        }
        self.text(start, self.at.min(self.src.len()))
    }

    /// Skips past the `}` closing a block whose `{` is consumed, over nested blocks and strings.
    fn skip_block(&mut self) {
        let mut depth = 1;
        while !self.done() && depth > 0 {
            let c = self.peek();
            if c == '"' || c == '\'' {
                // Skipped text reports nothing: the rule it is in is already reported or skipped.
                let q = c;
                self.at += 1;
                while !self.done() && self.peek() != q {
                    self.at += if self.peek() == '\\' { 2 } else { 1 };
                }
                self.at += 1;
                continue;
            }
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
            }
            self.at += 1;
        }
    }

    /// A quoted string, its quotes consumed and dropped.
    fn quoted(&mut self, cx: &mut Cx) -> String {
        let q = self.peek();
        let start = self.at;
        self.at += 1;
        let mut out = String::new();
        while !self.done() && self.peek() != q {
            if self.peek() == '\\' && self.at + 1 < self.src.len() {
                out.push(self.src[self.at + 1]);
                self.at += 2;
                continue;
            }
            out.push(self.peek());
            self.at += 1;
        }
        if self.done() {
            self.report(cx, Severity::Error, "a string is not closed", start);
        } else {
            self.at += 1;
        }
        out
    }

    fn ident(&mut self) -> String {
        let start = self.at;
        while !self.done() && is_ident(self.peek()) {
            self.at += 1;
        }
        self.text(start, self.at)
    }

    fn skip_space(&mut self, cx: &mut Cx) {
        while !self.done() {
            let c = self.peek();
            if is_space(c) {
                self.at += 1;
            } else if c == '/' && self.code(self.at + 1) == Some('*') {
                match self.find("*/", self.at + 2) {
                    None => {
                        self.report(cx, Severity::Error, "a comment is not closed", self.at);
                        self.at = self.src.len();
                    }
                    Some(end) => self.at = end + 2,
                }
            } else {
                break;
            }
        }
    }

    fn find(&self, needle: &str, from: usize) -> Option<usize> {
        let n: Vec<char> = needle.chars().collect();
        (from..self.src.len().saturating_sub(n.len() - 1))
            .find(|&i| self.src[i..i + n.len()] == n[..])
    }

    fn text(&self, from: usize, to: usize) -> String {
        self.src[from.min(to)..to].iter().collect()
    }

    fn done(&self) -> bool {
        self.at >= self.src.len()
    }

    fn peek(&self) -> char {
        self.src[self.at]
    }

    fn code(&self, i: usize) -> Option<char> {
        self.src.get(i).copied()
    }

    // --- Positions ---

    fn report(&self, cx: &mut Cx, severity: Severity, message: &str, index: usize) {
        let d = match &self.anchor {
            Some(a) => Diagnostic {
                severity,
                message: format!("{message} (in an @include)"),
                line: a.line,
                column: a.column,
                file: a.file.clone(),
            },
            None => Diagnostic {
                severity,
                message: message.to_string(),
                line: self.line_of(index),
                column: self.column_of(index),
                file: if self.imported {
                    self.file.clone()
                } else {
                    None
                },
            },
        };
        cx.sheet.diagnostics.push(d);
    }

    fn line_of(&self, index: usize) -> u32 {
        self.line_starts.partition_point(|&s| s <= index) as u32
    }

    fn column_of(&self, index: usize) -> u32 {
        (index - self.line_starts[self.line_of(index) as usize - 1] + 1) as u32
    }
}

/// `nested`'s selectors as text, composed with each of `parents`; `nested` alone at the top.
fn compose(parents: Option<&[String]>, nested: &str) -> Vec<String> {
    let own = value::split(nested, ',');
    let Some(parents) = parents else { return own };
    let mut out = Vec::new();
    for p in parents {
        // A parent with combinators keeps its meaning where it is put through :is().
        let wrapped = if p
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '>' | '+' | '~'))
        {
            format!(":is({p})")
        } else {
            p.clone()
        };
        for n in &own {
            if n.contains('&') {
                out.push(n.replace('&', &wrapped));
            } else {
                out.push(format!("{p} {n}"));
            }
        }
    }
    out
}

fn is_root(c: &Compound) -> bool {
    c.type_name.is_none()
        && c.id.is_none()
        && c.classes.is_empty()
        && c.pseudos.len() == 1
        && c.pseudos[0] == Pseudo::Root
}

/// The selectors of the rule a nested at-rule stands in.
fn parent_selectors(scope: &Scope) -> Vec<Selector> {
    scope
        .parents
        .as_ref()
        .and_then(|p| SelectorReader::new(&p.join(", "), 0).list(false).ok())
        .unwrap_or_default()
}

/// `"a.css" media`, `'a.css'`, `url(a.css) media` or `a.css`: the path and what follows it.
fn import_target(prelude: &str) -> Option<(String, String)> {
    let mut s = prelude;
    if let Some(rest) = s.strip_prefix("url(") {
        s = rest.trim_start();
    }
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '\'');
    if let Some(q) = quote {
        s = &s[1..];
        let end = s.find(|c: char| c == '"' || c == '\'' || c == ')' || c.is_whitespace())?;
        if end == 0 || !s[end..].starts_with(q) {
            return None;
        }
        let path = s[..end].to_string();
        let rest = s[end + 1..].trim_start();
        let rest = rest.strip_prefix(')').unwrap_or(rest).trim();
        return Some((path, rest.to_string()));
    }
    let end = s
        .find(|c: char| c == '"' || c == '\'' || c == ')' || c.is_whitespace())
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let path = s[..end].to_string();
    let rest = s[end..].trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim();
    Some((path, rest.to_string()))
}

/// `$name: value` as a mixin's named argument.
fn named_argument(a: &str) -> Option<(String, String)> {
    let rest = a.strip_prefix('$')?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let after = rest[end..].trim_start().strip_prefix(':')?;
    let value = after.trim();
    (!value.is_empty()).then(|| (name.to_string(), value.to_string()))
}

/// The value before a trailing `!important` (any case, spaces allowed after the `!`).
fn strip_important(value: &str) -> Option<&str> {
    let bang = value.rfind('!')?;
    value[bang + 1..]
        .trim_start()
        .eq_ignore_ascii_case("important")
        .then(|| &value[..bang])
}

fn is_property_name(name: &str) -> bool {
    let rest = name.strip_prefix("--").unwrap_or(name);
    let mut chars = rest.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '-')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The number at the start of `s`, ignoring what follows it: NaN when there is none.
fn parse_float_prefix(s: &str) -> f64 {
    let s = s.trim_start();
    let mut end = 0;
    for (i, _) in s
        .char_indices()
        .skip(1)
        .chain(std::iter::once((s.len(), ' ')))
    {
        if s[..i].parse::<f64>().is_ok() {
            end = i;
        }
    }
    if end == 0 {
        f64::NAN
    } else {
        s[..end].parse().unwrap_or(f64::NAN)
    }
}

/// Reads a selector list, the text before a rule's `{`, at `base` in the
/// sheet. Fails for one it cannot read, which skips the rule.
struct SelectorReader {
    text: Vec<char>,
    base: usize,
    at: usize,
}

type Read<T> = Result<T, Failure>;

impl SelectorReader {
    fn new(text: &str, base: usize) -> Self {
        SelectorReader {
            text: text.chars().collect(),
            base,
            at: 0,
        }
    }

    /// Comma-separated selectors to the end; `relative` lets each start with a combinator, as in `:has()`.
    fn list(&mut self, relative: bool) -> Read<Vec<Selector>> {
        let mut out = Vec::new();
        loop {
            out.push(self.complex(relative)?);
            self.space();
            if self.done() {
                break;
            }
            if self.peek() == ',' {
                self.at += 1;
                continue;
            }
            return self.fail(format!("unexpected \"{}\" in a selector", self.peek()));
        }
        Ok(out)
    }

    fn complex(&mut self, relative: bool) -> Read<Selector> {
        self.space();
        let mut leading = None;
        if relative {
            leading = self.combinator();
            self.space();
        }
        let mut compounds = vec![self.compound()?];
        let mut combinators = Vec::new();
        loop {
            let had_space = self.space();
            if self.done() || self.peek() == ',' || self.peek() == ')' {
                break;
            }
            let c = match self.combinator() {
                Some(c) => c,
                None if !had_space => {
                    return self.fail(format!("unexpected \"{}\" in a selector", self.peek()));
                }
                None => Combinator::Descendant,
            };
            self.space();
            combinators.push(c);
            compounds.push(self.compound()?);
        }
        Ok(Selector {
            compounds,
            combinators,
            leading,
        })
    }

    fn combinator(&mut self) -> Option<Combinator> {
        if self.done() {
            return None;
        }
        let c = match self.peek() {
            '>' => Combinator::Child,
            '+' => Combinator::NextSibling,
            '~' => Combinator::LaterSibling,
            _ => return None,
        };
        self.at += 1;
        Some(c)
    }

    fn compound(&mut self) -> Read<Compound> {
        let mut c = Compound::default();
        let start = self.at;
        if !self.done() && self.peek() == '*' {
            self.at += 1;
        } else if !self.done() && is_ident(self.peek()) && !self.peek().is_ascii_digit() {
            c.type_name = Some(self.name().to_lowercase());
        }
        while !self.done() {
            match self.peek() {
                '#' => {
                    self.at += 1;
                    let n = self.name();
                    c.id = Some(self.need(n, "an id after #")?);
                }
                '.' => {
                    self.at += 1;
                    let n = self.name();
                    c.classes.push(self.need(n, "a class name after .")?);
                }
                '[' => {
                    self.at += 1;
                    c.attributes.push(self.attribute()?);
                }
                ':' => {
                    self.at += 1;
                    if !self.done() && self.peek() == ':' {
                        self.at += 1;
                        let pe = self.name().to_lowercase();
                        if pe != "placeholder" {
                            return self.fail(format!("::{pe} is not supported; ::placeholder is"));
                        }
                        c.pseudo_element = Some(pe);
                    } else {
                        c.pseudos.push(self.pseudo()?);
                    }
                }
                _ => break,
            }
        }
        if self.at == start {
            return self.fail(if self.done() {
                "a selector is missing".to_string()
            } else {
                format!("unexpected \"{}\" in a selector", self.peek())
            });
        }
        Ok(c)
    }

    /// `name]`, or `name op value]` with the value quoted or a bare word; the `[` is consumed.
    fn attribute(&mut self) -> Read<Attribute> {
        self.space();
        let n = self.name();
        let n = self.need(n, "an attribute name after [")?.to_lowercase();
        self.space();
        if !self.done() && self.peek() == ']' {
            self.at += 1;
            return Ok(Attribute {
                name: n,
                op: None,
                value: None,
            });
        }
        let mut op = String::new();
        if !self.done() && "~|^$*".contains(self.peek()) {
            op.push(self.peek());
            self.at += 1;
        }
        if self.done() || self.peek() != '=' {
            return self.fail(format!("expected =, ~=, |=, ^=, $= or *= in [{n}]"));
        }
        self.at += 1;
        op.push('=');
        self.space();
        let value = if !self.done() && (self.peek() == '"' || self.peek() == '\'') {
            let q = self.peek();
            self.at += 1;
            let start = self.at;
            while !self.done() && self.peek() != q {
                self.at += 1;
            }
            let v: String = self.text[start..self.at].iter().collect();
            if self.done() {
                return self.fail("a quoted attribute value is not closed".into());
            }
            self.at += 1;
            v
        } else {
            let w = self.name();
            self.need(w, &format!("a value in [{n}{op}]"))?
        };
        self.space();
        // A case-insensitive flag is accepted and has no effect on these values.
        if !self.done()
            && (self.peek() == 'i' || self.peek() == 's')
            && self.at + 1 < self.text.len()
        {
            self.at += 1;
        }
        self.space();
        if self.done() || self.peek() != ']' {
            return self.fail(format!("expected ] after [{n}{op}\"{value}\""));
        }
        self.at += 1;
        Ok(Attribute {
            name: n,
            op: Some(op),
            value: Some(value),
        })
    }

    fn pseudo(&mut self) -> Read<Pseudo> {
        let name_at = self.at;
        let n = self.name().to_lowercase();
        if !self.done() && self.peek() == '(' {
            self.at += 1;
            let (inner, inner_at) = self.argument()?;
            return Ok(match n.as_str() {
                "not" => Pseudo::Not(self.nested(&inner, inner_at, false)?),
                "is" | "matches" => Pseudo::Is(self.nested(&inner, inner_at, false)?),
                "where" => Pseudo::Where(self.nested(&inner, inner_at, false)?),
                "has" => Pseudo::Has(self.nested(&inner, inner_at, true)?),
                "nth-child" => Pseudo::NthChild(self.nth(&inner)?),
                "nth-last-child" => Pseudo::NthLastChild(self.nth(&inner)?),
                "nth-of-type" => Pseudo::NthOfType(self.nth(&inner)?),
                "nth-last-of-type" => Pseudo::NthLastOfType(self.nth(&inner)?),
                _ => {
                    return self.fail_at(
                        format!(":{n}() is not a pseudo-class this supports"),
                        name_at,
                    );
                }
            });
        }
        Ok(match n.as_str() {
            "root" => Pseudo::Root,
            "empty" => Pseudo::Empty,
            "first-child" => Pseudo::FirstChild,
            "last-child" => Pseudo::LastChild,
            "only-child" => Pseudo::OnlyChild,
            "first-of-type" => Pseudo::FirstOfType,
            "last-of-type" => Pseudo::LastOfType,
            "only-of-type" => Pseudo::OnlyOfType,
            "before" | "after" | "first-line" | "first-letter" => {
                return self.fail_at(
                    format!(":{n} is a pseudo-element, which is not supported"),
                    name_at,
                );
            }
            s if STATES.contains(&s) => Pseudo::State(n),
            _ => return self.fail_at(format!(":{n} is not a pseudo-class this supports"), name_at),
        })
    }

    /// The text inside a functional pseudo-class's parentheses, the `)` consumed.
    fn argument(&mut self) -> Read<(String, usize)> {
        let start = self.at;
        let mut depth = 1;
        while !self.done() {
            match self.peek() {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.at += 1;
        }
        if self.done() {
            return self.fail_at("a ( is not closed".into(), start - 1);
        }
        let inner: String = self.text[start..self.at].iter().collect();
        self.at += 1;
        Ok((inner, start))
    }

    fn nested(&self, inner: &str, inner_at: usize, relative: bool) -> Read<Vec<Selector>> {
        if inner.trim().is_empty() {
            return self.fail_at("a selector is missing".into(), inner_at);
        }
        SelectorReader::new(inner, self.base + inner_at).list(relative)
    }

    /// `odd`, `even`, `b`, `an`, `an+b`, `-n+b`, spaces allowed around the sign.
    fn nth(&self, s: &str) -> Read<Nth> {
        let t = s.trim().to_lowercase().replace(' ', "");
        if t == "odd" {
            return Ok(Nth { a: 2, b: 1 });
        }
        if t == "even" {
            return Ok(Nth { a: 2, b: 0 });
        }
        if let Some(n) = t.find('n') {
            let (a, b) = (&t[..n], &t[n + 1..]);
            let a_ok = a
                .strip_prefix(['+', '-'])
                .unwrap_or(a)
                .chars()
                .all(|c| c.is_ascii_digit());
            let b_ok = b.is_empty()
                || (b.starts_with(['+', '-'])
                    && b.len() > 1
                    && b[1..].chars().all(|c| c.is_ascii_digit()));
            if a_ok && b_ok {
                let a = match a {
                    "" | "+" => 1,
                    "-" => -1,
                    _ => a.parse().unwrap_or(0),
                };
                let b = if b.is_empty() {
                    0
                } else {
                    b.parse().unwrap_or(0)
                };
                return Ok(Nth { a, b });
            }
        }
        let digits = t.strip_prefix(['+', '-']).unwrap_or(&t);
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Nth {
                a: 0,
                b: t.parse().unwrap_or(0),
            });
        }
        self.fail(format!(
            "\"{s}\" is not an an+b pattern, such as 2n+1, odd or 3"
        ))
    }

    /// An identifier, with `\` escaping the next character, as Tailwind-style class names need: `.hover\:bg-x`.
    fn name(&mut self) -> String {
        let mut out = String::new();
        while !self.done() {
            let c = self.peek();
            if c == '\\' && self.at + 1 < self.text.len() {
                out.push(self.text[self.at + 1]);
                self.at += 2;
            } else if is_ident(c) {
                out.push(c);
                self.at += 1;
            } else {
                break;
            }
        }
        out
    }

    fn need(&self, s: String, what: &str) -> Read<String> {
        if s.is_empty() {
            self.fail(format!("expected {what}"))
        } else {
            Ok(s)
        }
    }

    /// Skips whitespace and comments; true if there was any.
    fn space(&mut self) -> bool {
        let start = self.at;
        while !self.done() {
            let c = self.peek();
            if is_space(c) {
                self.at += 1;
            } else if c == '/' && self.text.get(self.at + 1) == Some(&'*') {
                let end = (self.at + 2..self.text.len().saturating_sub(1))
                    .find(|&i| self.text[i] == '*' && self.text[i + 1] == '/');
                self.at = end.map_or(self.text.len(), |e| e + 2);
            } else {
                break;
            }
        }
        self.at > start
    }

    fn done(&self) -> bool {
        self.at >= self.text.len()
    }

    fn peek(&self) -> char {
        self.text[self.at]
    }

    fn fail<T>(&self, message: String) -> Read<T> {
        self.fail_at(message, self.at)
    }

    fn fail_at<T>(&self, message: String, index: usize) -> Read<T> {
        Err(Failure {
            message,
            at: self.base + index,
        })
    }
}
