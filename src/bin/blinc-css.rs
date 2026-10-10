//! Compiles CSS ahead of time, for builds that cannot load blinc_abi: a
//! compile-time step runs it to check a sheet and embed its compiled form.
//!
//! ```text
//! blinc-css <in.css> [-o out.bcss] [--json] [--classes] [--manifest]
//! ```
//!
//! Diagnostics go to stderr as `file:line:column: severity: message`, and
//! the exit status is 1 when any is an error. `-o` writes the compiled sheet;
//! `--json` prints the parsed sheet as JSON; `--classes` prints the class
//! names its selectors use, one a line; `--manifest` prints, as JSON, the
//! files it imported and those class names: what a build tracks and checks.

use blinc_abi::css;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut input, mut output, mut json, mut classes, mut manifest) =
        (None, None, false, false, false);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                output = args.get(i).cloned();
            }
            "--json" => json = true,
            "--classes" => classes = true,
            "--manifest" => manifest = true,
            "-h" | "--help" => {
                eprintln!("blinc-css <in.css> [-o out.bcss] [--json] [--classes] [--manifest]");
                return ExitCode::SUCCESS;
            }
            a if input.is_none() && !a.starts_with('-') => input = Some(a.to_string()),
            a => {
                eprintln!("blinc-css: unknown argument {a}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("blinc-css <in.css> [-o out.bcss] [--json] [--classes] [--manifest]");
        return ExitCode::from(2);
    };
    let source = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("blinc-css: {input}: {e}");
            return ExitCode::from(2);
        }
    };
    // An import is read relative to the file importing it.
    let mut load = |path: &str, from: Option<&str>| {
        let file = match from {
            Some(f) if !Path::new(path).is_absolute() => Path::new(f)
                .parent()
                .unwrap_or(Path::new(""))
                .join(path)
                .to_string_lossy()
                .into_owned(),
            _ => path.to_string(),
        };
        std::fs::read_to_string(&file).ok().map(|s| (s, file))
    };
    let sheet = css::parse(&source, Some(&input), &mut load);
    let report = sheet.report(Some(&input));
    if !report.is_empty() {
        eprintln!("{report}");
    }
    if let Some(out) = output
        && let Err(e) = std::fs::write(&out, css::compiled::encode(&sheet))
    {
        eprintln!("blinc-css: {out}: {e}");
        return ExitCode::from(2);
    }
    if json {
        println!("{}", css::json::to_json(&sheet));
    }
    if classes {
        for c in sheet.class_names() {
            println!("{c}");
        }
    }
    if manifest {
        let strings = |items: Vec<&str>| {
            let quoted: Vec<String> = items.iter().map(|s| css::json::string(s)).collect();
            format!("[{}]", quoted.join(","))
        };
        let imports = sheet.imports.iter().map(|&a| sheet.str(a)).collect();
        let names = sheet.class_names();
        println!(
            "{{\"imports\":{},\"classes\":{}}}",
            strings(imports),
            strings(names.iter().map(|c| c.as_ref()).collect())
        );
    }
    if sheet.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
