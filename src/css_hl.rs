//! The shared CSS engine for HashLink: one [`Styles`] over the process's
//! layout tree. The host describes its elements and their states, adds and
//! removes sheets, and restyles; it then reads each changed node's resolved
//! declarations and applies them itself, layout included, so the cascade
//! writes nothing to the tree.
//!
//! Lists cross as one UTF-8 string: records split by U+0001, a name and its
//! value by U+0002, and an element's attributes and declarations by U+0003.

use crate::context::PropValue;
use crate::css::cascade::{Element, SheetId, States};
use crate::css::styled::{Host, Styles};
use crate::css::{self, MediaEnvironment, Severity};
use crate::hl::{handle_mut, handle_ref, into_handle, string_from, string_to_hl};
use crate::tree::Tree;
use blinc_layout::tree::LayoutNodeId;
use hl_abi::{define_prim, vbyte};
use std::ffi::c_void;
use std::path::Path;

const RECORD: char = '\u{1}';
const PAIR: char = '\u{2}';
const ITEM: char = '\u{3}';

/// The process's layout tree, as the cascade walks it. Layout is the host's to apply.
struct TreeHost<'a>(&'a Tree);

impl Host for TreeHost<'_> {
    fn live(&self, node: u64) -> bool {
        self.0.layout.node_exists(LayoutNodeId::from_raw(node))
    }
    fn parent(&self, node: u64) -> Option<u64> {
        self.0
            .layout
            .ancestors(LayoutNodeId::from_raw(node))
            .first()
            .map(|p| p.to_raw())
    }
    fn children(&self, node: u64) -> Vec<u64> {
        self.0
            .layout
            .children(LayoutNodeId::from_raw(node))
            .into_iter()
            .map(|c| c.to_raw())
            .collect()
    }
    fn apply_layout(&mut self, _: u64, _: &[(i32, PropValue<'_>)]) -> Result<(), &'static str> {
        Ok(())
    }
    fn lays_out(&self) -> bool {
        false
    }
}

unsafe fn styles<'a>(h: *mut c_void) -> Option<&'a mut Styles> {
    unsafe { handle_mut::<Styles>(h) }
}

#[unsafe(no_mangle)]
pub extern "C" fn hl_blinc_css_new() -> *mut c_void {
    into_handle(Styles::new())
}
define_prim!(hlp_blinc_css_new, hl_blinc_css_new, "P_Xblinc_css_");

/// Each diagnostic as `error|warning line:column file message`, one per record.
fn report(sheet: &css::Stylesheet) -> String {
    let mut out = String::new();
    for d in &sheet.diagnostics {
        if !out.is_empty() {
            out.push(RECORD);
        }
        let severity = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        out.push_str(&format!(
            "{severity}{PAIR}{}{PAIR}{}{PAIR}{}{PAIR}{}",
            d.line,
            d.column,
            d.file.as_deref().unwrap_or(""),
            d.message
        ));
    }
    out
}

/// Parses `source` and adds it at position `at` (past the end for last).
/// Imports are read relative to the file importing them. Writes the sheet's
/// id to `id`, or -1, and returns its diagnostics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_add(
    h: *mut c_void,
    source: *const vbyte,
    file: *const vbyte,
    at: i32,
    id: *mut i32,
) -> *mut vbyte {
    let Some(s) = (unsafe { styles(h) }) else {
        return string_to_hl("");
    };
    let source = unsafe { string_from(source) };
    let file = unsafe { string_from(file) };
    let file = (!file.is_empty()).then_some(file);
    let mut load = |path: &str, from: Option<&str>| {
        let target = match from {
            Some(f) if !Path::new(path).is_absolute() => Path::new(f)
                .parent()
                .unwrap_or(Path::new(""))
                .join(path)
                .to_string_lossy()
                .into_owned(),
            _ => path.to_string(),
        };
        std::fs::read_to_string(&target).ok().map(|t| (t, target))
    };
    let sheet = css::parse(&source, file.as_deref(), &mut load);
    let diagnostics = report(&sheet);
    let sheet_id = s.cascade_mut().insert(at.max(0) as usize, sheet);
    if !id.is_null() {
        unsafe { *id = sheet_id.0 as i32 };
    }
    string_to_hl(&diagnostics)
}
define_prim!(hlp_blinc_css_add, hl_blinc_css_add, "PXblinc_css_BBiB_B");

/// Adds a compiled sheet of `len` bytes at `at`; its id, or -1 for bytes
/// that do not decode.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_add_compiled(
    h: *mut c_void,
    bytes: *const vbyte,
    len: i32,
    at: i32,
) -> i32 {
    let Some(s) = (unsafe { styles(h) }) else {
        return -1;
    };
    if bytes.is_null() || len < 0 {
        return -1;
    }
    let data = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
    match css::compiled::decode(data) {
        Ok(sheet) => s.cascade_mut().insert(at.max(0) as usize, sheet).0 as i32,
        Err(_) => -1,
    }
}
define_prim!(
    hlp_blinc_css_add_compiled,
    hl_blinc_css_add_compiled,
    "PXblinc_css_Bii_i"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_remove(h: *mut c_void, id: i32) -> i32 {
    unsafe { styles(h) }.is_some_and(|s| s.cascade_mut().remove(SheetId(id as u32))) as i32
}
define_prim!(hlp_blinc_css_remove, hl_blinc_css_remove, "PXblinc_css_i_i");

/// The theme's variables, `name` without `--` paired with its value, which
/// `var()` reads after the sheets'.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_set_theme(h: *mut c_void, vars: *const vbyte) {
    let Some(s) = (unsafe { styles(h) }) else {
        return;
    };
    let text = unsafe { string_from(vars) };
    let pairs: Vec<(&str, &str)> = text
        .split(RECORD)
        .filter_map(|r| r.split_once(PAIR))
        .collect();
    s.cascade_mut().set_theme(&pairs);
}
define_prim!(
    hlp_blinc_css_set_theme,
    hl_blinc_css_set_theme,
    "PXblinc_css_B_v"
);

/// The viewport media queries and viewport units read, the color scheme, and
/// the root font size `rem` is of.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_set_environment(
    h: *mut c_void,
    width: f64,
    height: f64,
    dark: i32,
    root_font_size: f64,
) {
    let Some(s) = (unsafe { styles(h) }) else {
        return;
    };
    s.set_environment(MediaEnvironment {
        width,
        height,
        dark: dark != 0,
    });
    s.cascade_mut().set_root_font_size(root_font_size);
}
define_prim!(
    hlp_blinc_css_set_environment,
    hl_blinc_css_set_environment,
    "PXblinc_css_ddid_v"
);

/// Describes `node`: its types (own first), id, classes, attributes and
/// inline declarations, and whether a layout made it. `desc` holds six
/// records: types and classes split by spaces; attributes and declarations
/// as name-value pairs split by U+0003; `1` for anonymous.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_set_element(h: *mut c_void, node: u64, desc: *const vbyte) {
    let Some(s) = (unsafe { styles(h) }) else {
        return;
    };
    let text = unsafe { string_from(desc) };
    let mut fields = text.split(RECORD);
    let mut next = || fields.next().unwrap_or("");
    let (types, id, classes, attributes, inline, anonymous) =
        (next(), next(), next(), next(), next(), next());
    let mut words = |t: &str| {
        t.split(' ')
            .filter(|w| !w.is_empty())
            .map(|w| s.intern(w))
            .collect::<Vec<_>>()
    };
    let types = words(types);
    let classes = words(classes);
    let id = (!id.is_empty()).then(|| s.intern(id));
    let pairs = |t: &str| {
        t.split(ITEM)
            .filter_map(|p| p.split_once(PAIR))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
    };
    let attributes = pairs(attributes)
        .into_iter()
        .map(|(k, v)| (s.intern(&k), s.intern(&v)))
        .collect();
    let inline = pairs(inline)
        .into_iter()
        .map(|(k, v)| (s.intern(&k), v))
        .collect();
    s.set_element(
        node,
        Element {
            types,
            id,
            classes,
            attributes,
            inline,
            states: States::default(),
            anonymous: anonymous == "1",
        },
    );
}
define_prim!(
    hlp_blinc_css_set_element,
    hl_blinc_css_set_element,
    "PXblinc_css_lB_v"
);

/// The bit of state `name` in `set_states`'s mask, or 0 for a state the engine does not know.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_state_bit(name: *const vbyte) -> i32 {
    States::bit(&unsafe { string_from(name) }).map_or(0, |b| b as i32)
}
define_prim!(hlp_blinc_css_state_bit, hl_blinc_css_state_bit, "PB_i");

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_set_states(h: *mut c_void, node: u64, mask: i32) {
    if let Some(s) = unsafe { styles(h) } {
        s.set_states(node, States(mask as u32));
    }
}
define_prim!(
    hlp_blinc_css_set_states,
    hl_blinc_css_set_states,
    "PXblinc_css_li_v"
);

/// `node` was placed under a new parent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_moved(h: *mut c_void, node: u64) {
    if let Some(s) = unsafe { styles(h) } {
        s.moved(node);
    }
}
define_prim!(hlp_blinc_css_moved, hl_blinc_css_moved, "PXblinc_css_l_v");

/// A child was placed under `parent`, removed from it, or moved among its siblings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_children_changed(h: *mut c_void, parent: u64) {
    if let Some(s) = unsafe { styles(h) } {
        s.children_changed(parent);
    }
}
define_prim!(
    hlp_blinc_css_children_changed,
    hl_blinc_css_children_changed,
    "PXblinc_css_l_v"
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_forget(h: *mut c_void, node: u64) {
    if let Some(s) = unsafe { styles(h) } {
        s.forget(node);
    }
}
define_prim!(hlp_blinc_css_forget, hl_blinc_css_forget, "PXblinc_css_l_v");

/// Restyles what changed under `root` of the tree, and returns how many
/// nodes' styles changed; `take_changed` lists them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_restyle(h: *mut c_void, t: *mut c_void, root: u64) -> i32 {
    let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
        return 0;
    };
    s.restyle_host(&mut TreeHost(tree), root);
    s.changed_count() as i32
}
define_prim!(
    hlp_blinc_css_restyle,
    hl_blinc_css_restyle,
    "PXblinc_css_Xblinc_tree_l_i"
);

/// Writes the nodes whose styles changed, parents first, to `out`, at most
/// `capacity`, and forgets those; the rest wait for the next call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_take_changed(
    h: *mut c_void,
    out: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(s) = (unsafe { styles(h) }) else {
        return 0;
    };
    let changed = s.take_changed_upto(capacity.max(0) as usize);
    let n = changed.len();
    let out = out as *mut u64;
    for (i, raw) in changed.iter().take(n).enumerate() {
        unsafe { out.add(i).write_unaligned(*raw) };
    }
    n as i32
}
define_prim!(
    hlp_blinc_css_take_changed,
    hl_blinc_css_take_changed,
    "PXblinc_css_Bi_i"
);

/// Writes the states selectors began to test since this was last called,
/// as node and state-bit pairs, at most `capacity`, and forgets those (the
/// rest wait for the next call): the host
/// tells `set_states` when one of these changes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_take_watched(
    h: *mut c_void,
    nodes: *mut vbyte,
    bits: *mut vbyte,
    capacity: i32,
) -> i32 {
    let Some(s) = (unsafe { styles(h) }) else {
        return 0;
    };
    let watched = s.take_watched_upto(capacity.max(0) as usize);
    let n = watched.len();
    let (nodes, bits) = (nodes as *mut u64, bits as *mut i32);
    for (i, (node, bit)) in watched.iter().take(n).enumerate() {
        unsafe {
            nodes.add(i).write_unaligned(*node);
            bits.add(i).write_unaligned(*bit as i32);
        }
    }
    n as i32
}
define_prim!(
    hlp_blinc_css_take_watched,
    hl_blinc_css_take_watched,
    "PXblinc_css_BBi_i"
);

/// `node`'s style: its resolved declarations (`var()`s replaced), its values
/// (inherited ones and custom properties included), then its font size in
/// pixels, as three records of name-value pairs split by U+0003.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_css_style(h: *mut c_void, node: u64) -> *mut vbyte {
    let Some(s) = (unsafe { handle_ref::<Styles>(h) }) else {
        return string_to_hl("");
    };
    let Some(c) = s.computed(node) else {
        return string_to_hl("");
    };
    let list = |pairs: &[(css::Atom, String)]| {
        let mut out = String::new();
        for (k, v) in pairs {
            if !out.is_empty() {
                out.push(ITEM);
            }
            out.push_str(s.cascade().str(*k));
            out.push(PAIR);
            out.push_str(v);
        }
        out
    };
    let text = format!(
        "{}{RECORD}{}{RECORD}{}",
        list(&c.resolved),
        list(&c.values),
        c.font_size
    );
    string_to_hl(&text)
}
define_prim!(hlp_blinc_css_style, hl_blinc_css_style, "PXblinc_css_l_B");

#[cfg(test)]
mod tests {
    use super::*;

    fn element(s: &mut Styles, types: &[&str], classes: &[&str]) -> Element {
        Element {
            types: types.iter().map(|t| s.intern(t)).collect(),
            classes: classes.iter().map(|c| s.intern(c)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_tree_is_styled_and_its_changes_listed() {
        let mut tree = Tree::new();
        let root = tree.layout.create_node(Default::default());
        let a = tree.layout.create_node(Default::default());
        let b = tree.layout.create_node(Default::default());
        tree.layout.add_child(root, a);
        tree.layout.add_child(root, b);
        let mut s = Styles::new();
        let mut load = |_: &str, _: Option<&str>| None;
        s.cascade_mut().push(css::parse(
            ".list > .row { height: 10px; color: red } .row:last-child { color: blue }",
            None,
            &mut load,
        ));
        let e = element(&mut s, &["div"], &["list"]);
        s.set_element(root.to_raw(), e);
        for n in [a, b] {
            let e = element(&mut s, &["div"], &["row"]);
            s.set_element(n.to_raw(), e);
        }
        s.restyle_host(&mut TreeHost(&tree), root.to_raw());
        assert_eq!(
            s.take_changed(),
            vec![root.to_raw(), a.to_raw(), b.to_raw()]
        );
        let color = s.intern("color");
        assert_eq!(
            s.computed(a.to_raw()).and_then(|c| c.value(color)),
            Some("red")
        );
        assert_eq!(
            s.computed(b.to_raw()).and_then(|c| c.value(color)),
            Some("blue")
        );
        // The host applies layout: height stays a resolved declaration, written to nothing.
        let height = s.intern("height");
        assert!(
            s.computed(a.to_raw())
                .unwrap()
                .resolved
                .iter()
                .any(|(k, v)| *k == height && v == "10px")
        );
        assert_eq!(
            tree.layout.get_style(a).unwrap().size.height,
            taffy::Dimension::auto()
        );

        // Nothing changed: nothing listed.
        s.restyle_host(&mut TreeHost(&tree), root.to_raw());
        assert!(s.take_changed().is_empty());
    }

    #[test]
    fn a_forest_is_styled_from_each_top_node() {
        let mut tree = Tree::new();
        let top = tree.layout.create_node(Default::default());
        let child = tree.layout.create_node(Default::default());
        let other = tree.layout.create_node(Default::default());
        tree.layout.add_child(top, child);
        let mut s = Styles::new();
        let mut load = |_: &str, _: Option<&str>| None;
        s.cascade_mut().push(css::parse(
            ":root { color: red } div div { color: blue }",
            None,
            &mut load,
        ));
        for n in [top, child, other] {
            let e = element(&mut s, &["div"], &[]);
            s.set_element(n.to_raw(), e);
        }
        s.restyle_host(&mut TreeHost(&tree), 0u64);
        let color = s.intern("color");
        let value = |s: &Styles, n: LayoutNodeId| {
            s.computed(n.to_raw())
                .and_then(|c| c.value(color))
                .map(str::to_string)
        };
        assert_eq!(value(&s, top).as_deref(), Some("red"));
        assert_eq!(value(&s, other).as_deref(), Some("red"));
        assert_eq!(value(&s, child).as_deref(), Some("blue"));
        let mut changed = s.take_changed();
        changed.sort();
        let mut all = vec![top.to_raw(), child.to_raw(), other.to_raw()];
        all.sort();
        assert_eq!(changed, all);
    }
}
