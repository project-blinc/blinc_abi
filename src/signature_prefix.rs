//! HashLink compares a native's signature with its function type, and that string starts with `P`. A
//! primitive declared without it loads on Ash, which ignores the signature, and stops the standard
//! HashLink VM at startup. This reads the sources and checks that every signature carries it.

use std::{
    fs,
    path::{Path, PathBuf},
};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The arguments of a call whose opening parenthesis comes just before `text`, split at top-level commas.
fn arguments(text: &str) -> Vec<&str> {
    let (mut depth, mut start, mut quoted) = (0usize, 0usize, false);
    let mut args = Vec::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            _ if quoted => {}
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => {
                args.push(&text[start..i]);
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    args
}

#[test]
fn every_primitive_signature_starts_with_p() {
    let call = ["define", "_prim!("].concat();
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let (mut checked, mut missing) = (0, Vec::new());
    for file in files.iter().filter(|f| !f.ends_with("signature_prefix.rs")) {
        let text = fs::read_to_string(file).unwrap();
        let line_of = |at: usize| text[..at].matches('\n').count() + 1;
        for (at, _) in text.match_indices(&call) {
            let args = arguments(&text[at + call.len()..]);
            let Some(signature) = args.get(2).map(|a| a.trim()) else {
                continue;
            };
            // A macro parameter: the call that fills it in is checked where it is written.
            if signature.starts_with('$') {
                continue;
            }
            checked += 1;
            if !signature
                .strip_prefix('"')
                .is_some_and(|s| s.starts_with('P'))
            {
                missing.push(format!("{}:{}: {signature}", file.display(), line_of(at)));
            }
        }
        // `export_numeric!` lists its signatures as `hl_x / hlp_x = "Psig"`.
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if let (true, Some((_, signature))) = (
                line.starts_with("hl_") && line.contains(" / hlp_"),
                line.split_once(" = "),
            ) {
                checked += 1;
                if !signature
                    .trim_start()
                    .strip_prefix('"')
                    .is_some_and(|s| s.starts_with('P'))
                {
                    missing.push(format!("{}:{}: {signature}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(
        checked > 100,
        "found only {checked} signatures: the check no longer sees the sources"
    );
    assert!(
        missing.is_empty(),
        "signatures without the leading P:\n{}",
        missing.join("\n")
    );
}
