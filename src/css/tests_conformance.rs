//! The conformance ledger shared with Blinc's `blinc_css`: a verdict from each engine for each
//! declaration. This checks the `blinc_abi::css` column. The file lives in the Blinc repository
//! (`crates/blinc_css/tests/conformance/values.tsv`); set `BLINC_CSS_LEDGER` to point at another
//! copy. Without one the test says so and passes, as the repositories are checked out apart.

use super::layout::{Units, is_layout_property, layout_writes};
use super::paint::{is_paint_property, paint_writes};
use super::quantity::PaintUnits;

/// ok, reject, or unrouted for a property no write is made for (a host reads those itself).
fn verdict(name: &str, value: &str) -> &'static str {
    let ok = |r: bool| if r { "ok" } else { "reject" };
    if is_paint_property(name) {
        return ok(matches!(
            paint_writes(name, Some(value), &PaintUnits::default()),
            Some(Ok(_))
        ));
    }
    if is_layout_property(name) {
        return ok(matches!(
            layout_writes(name, Some(value), &Units::default()),
            Some(Ok(_))
        ));
    }
    "unrouted"
}

fn ledger() -> Option<String> {
    let path = std::env::var("BLINC_CSS_LEDGER").unwrap_or_else(|_| {
        format!(
            "{}/../Blinc/crates/blinc_css/tests/conformance/values.tsv",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    std::fs::read_to_string(path).ok()
}

#[test]
fn blinc_abi_css_matches_its_column_of_the_ledger() {
    let Some(ledger) = ledger() else {
        eprintln!("no conformance ledger next to this checkout: skipped");
        return;
    };
    let mut stale = Vec::new();
    let mut rows = 0;
    for line in ledger
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert!(cols.len() >= 4, "bad ledger row: {line}");
        rows += 1;
        let got = verdict(cols[0], cols[1]);
        if got != cols[3] {
            stale.push(format!(
                "{}: {} is {got}, the ledger says {}",
                cols[0], cols[1], cols[3]
            ));
        }
    }
    assert!(rows > 100, "the ledger is nearly empty: {rows} rows");
    assert!(stale.is_empty(), "stale ledger rows:\n{}", stale.join("\n"));
}
