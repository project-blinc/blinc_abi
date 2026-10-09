//! Reading CSS value text: splitting lists and calls, numbers and units.

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// `text` split at `separator` outside parentheses and quotes, each part
/// trimmed; split at spaces, the empty parts dropped.
pub fn split(text: &str, separator: char) -> Vec<String> {
    let space = separator == ' ';
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 && (if space { is_space(c) } else { c == separator }) => {
                out.push(text[start..i].trim().to_string());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(text[start..].trim().to_string());
    if space {
        out.retain(|s| !s.is_empty());
    }
    out
}

/// `name(args)` as its lower-case name and the text of its arguments; none
/// if `text` is not one call.
pub fn call(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    let open = t.find('(')?;
    let name = &t[..open];
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
        || !t.ends_with(')')
    {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    // The closing parenthesis must close the opening one.
    let mut depth = 0i32;
    for c in inner.chars() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth < 0 {
                return None;
            }
        }
    }
    (depth == 0).then(|| (name.to_ascii_lowercase(), inner.to_string()))
}

/// A number and its unit, lower case, `""` for none; none if `text` is not one.
pub fn dimension(text: &str) -> Option<(f64, String)> {
    let t = text.trim();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut any = i > digits;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        any |= i > frac;
    }
    if !any {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp {
            i = j;
        }
    }
    let unit = &t[i..];
    if !unit.chars().all(|c| c.is_ascii_alphabetic() || c == '%') {
        return None;
    }
    Some((t[..i].parse().ok()?, unit.to_ascii_lowercase()))
}

/// A plain number; an error naming `text` if it has a unit or is not one.
pub fn number(text: &str) -> Result<f64, String> {
    match dimension(text) {
        Some((v, u)) if u.is_empty() => Ok(v),
        _ => Err(format!("expected a number, not \"{text}\"")),
    }
}
