//! The shared CSS engine: one [`Styles`] over the process's layout tree. The host describes its elements and
//! their states, adds and removes sheets, and restyles; it then reads each changed node's resolved declarations
//! and applies them itself, layout included, so the cascade writes nothing to the tree. The operations here
//! take plain Rust values; the HashLink and Caribou adapters marshal into them.
//!
//! Lists cross as one UTF-8 string: records split by U+0001, a name and its value by U+0002, and an element's
//! attributes and declarations by U+0003.

use crate::context::PropValue;
use crate::css::cascade::{Element, SheetId, States};
use crate::css::styled::{Host, Styles};
use crate::css::{self, MediaEnvironment, Severity};
#[cfg(feature = "hashlink")]
use crate::hl::{handle_mut, handle_ref, into_handle, string_from, string_to_hl};
use crate::tree::Tree;
use blinc_layout::tree::LayoutNodeId;
#[cfg(feature = "hashlink")]
use hl_abi::{define_prim, vbyte};
#[cfg(feature = "hashlink")]
use std::ffi::c_void;
use std::path::Path;

pub(crate) const RECORD: char = '\u{1}';
pub(crate) const PAIR: char = '\u{2}';
pub(crate) const ITEM: char = '\u{3}';

/// The process's layout tree, as the cascade walks it. Layout is the host's to apply.
pub(crate) struct TreeHost<'a>(&'a mut Tree);

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
    fn set_containment(
        &mut self,
        node: u64,
        kind: css::conditional::ContainerType,
    ) -> Result<(), &'static str> {
        self.0
            .layout
            .set_containment(LayoutNodeId::from_raw(node), kind);
        Ok(())
    }
    fn container_size(&self, node: u64) -> Option<(f64, f64)> {
        self.0.layout.container_size(LayoutNodeId::from_raw(node))
    }
}

/// Each diagnostic as `error|warning line:column file message`, one per record.
pub(crate) fn report(sheet: &css::Stylesheet) -> String {
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

/// `source` parsed; `file`, when not empty, names it and is where its imports are read from.
pub(crate) fn parse_sheet(source: &str, file: &str) -> css::Stylesheet {
    parse_file(source, (!file.is_empty()).then_some(file))
}

fn parse_file(source: &str, file: Option<&str>) -> css::Stylesheet {
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
    css::parse(source, file, &mut load)
}

/// Parses `source` and adds it at position `at` (past the end for last). Imports are read relative to the
/// file importing them. Returns the sheet's id and its diagnostics.
pub(crate) fn add_source(s: &mut Styles, source: &str, file: &str, at: i32) -> (i32, String) {
    let sheet = parse_sheet(source, file);
    let diagnostics = report(&sheet);
    let id = s.cascade_mut().insert(at.max(0) as usize, sheet);
    (id.0 as i32, diagnostics)
}

/// A compiled sheet's bytes decoded; `None` for bytes that do not decode.
pub(crate) fn decode_sheet(data: &[u8]) -> Option<css::Stylesheet> {
    css::compiled::decode(data).ok()
}

/// The files a parsed sheet imported, directly or through another, one per record.
pub(crate) fn sheet_imports(s: &css::Stylesheet) -> String {
    s.imports
        .iter()
        .map(|&a| s.str(a))
        .collect::<Vec<_>>()
        .join(&RECORD.to_string())
}

/// Every declaration of a parsed sheet's rules, as `name`, line and column per record.
pub(crate) fn sheet_declared(s: &css::Stylesheet) -> String {
    let mut out = String::new();
    for rule in &s.rules {
        for d in s.rule_declarations(rule) {
            if !out.is_empty() {
                out.push(RECORD);
            }
            out.push_str(&format!(
                "{}{PAIR}{}{PAIR}{}",
                s.str(d.name),
                d.line,
                d.column
            ));
        }
    }
    out
}

/// Adds a parsed sheet at `at`; its id. The sheet stays the caller's, to add again.
pub(crate) fn add_parsed(s: &mut Styles, sheet: &css::Stylesheet, at: i32) -> i32 {
    s.cascade_mut().insert(at.max(0) as usize, sheet.clone()).0 as i32
}

/// Adds a compiled sheet at `at`; its id, or -1 for bytes that do not decode.
pub(crate) fn add_compiled(s: &mut Styles, data: &[u8], at: i32) -> i32 {
    match css::compiled::decode(data) {
        Ok(sheet) => s.cascade_mut().insert(at.max(0) as usize, sheet).0 as i32,
        Err(_) => -1,
    }
}

pub(crate) fn remove_sheet(s: &mut Styles, id: i32) -> i32 {
    s.cascade_mut().remove(SheetId(id as u32)) as i32
}

/// The `@keyframes` named `name` among the sheets in force, a later sheet's over an earlier's: a record per
/// frame, its offsets split by spaces, then after U+0002 its declarations as items of name, U+0004, value.
/// `None` when no sheet has one.
pub(crate) fn keyframes(s: &Styles, name: &str) -> Option<String> {
    let (sheet, k) = s.cascade().keyframes(name)?;
    let frames: Vec<String> = sheet
        .keyframe_list(k)
        .iter()
        .map(|f| {
            let offsets: Vec<String> = sheet
                .keyframe_offsets(f)
                .iter()
                .map(|o| css::json::number(*o))
                .collect();
            let declarations: Vec<String> = sheet
                .keyframe_declarations(f)
                .iter()
                .map(|d| format!("{}\u{4}{}", sheet.str(d.name), sheet.str(d.value)))
                .collect();
            format!(
                "{}{PAIR}{}",
                offsets.join(" "),
                declarations.join(&ITEM.to_string())
            )
        })
        .collect();
    Some(frames.join(&RECORD.to_string()))
}

/// `:root`'s custom property `name` (no `--`) among the sheets in force; `None` when none declares it.
pub(crate) fn root_variable(s: &Styles, name: &str) -> Option<String> {
    s.cascade().root_variable(name).map(|v| v.to_string())
}

/// The theme's variables, `name` without `--` paired with its value, which `var()` reads after the sheets'.
pub(crate) fn set_theme(s: &mut Styles, vars: &str) {
    let pairs: Vec<(&str, &str)> = vars
        .split(RECORD)
        .filter_map(|r| r.split_once(PAIR))
        .collect();
    s.cascade_mut().set_theme(&pairs);
}

/// The viewport media queries and viewport units read, the color scheme, and the root font size `rem` is of.
pub(crate) fn set_environment(
    s: &mut Styles,
    width: f64,
    height: f64,
    dark: bool,
    root_font_size: f64,
) {
    s.set_environment(MediaEnvironment {
        width,
        height,
        dark,
    });
    s.cascade_mut().set_root_font_size(root_font_size);
}

/// Describes `node`: its types (own first), id, classes, attributes and inline declarations, and whether a
/// layout made it. `desc` holds six records: types and classes split by spaces; attributes and declarations
/// as name-value pairs split by U+0003; `1` for anonymous.
pub(crate) fn set_element(s: &mut Styles, node: u64, desc: &str) {
    let mut fields = desc.split(RECORD);
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
            generated: false,
        },
    );
}

/// The bit of state `name` in `set_states`'s mask, or 0 for a state the engine does not know.
pub(crate) fn state_bit(name: &str) -> i32 {
    States::bit(name).map_or(0, |b| b as i32)
}

/// Restyles what changed under `root` of the tree, and returns how many nodes' styles changed.
pub(crate) fn restyle(s: &mut Styles, tree: &mut Tree, root: u64) -> i32 {
    let pending = s.changed_count();
    s.restyle_host(&mut TreeHost(tree), root);
    // Only native pattern state is ours to apply. Read it from the already resolved declarations; the host
    // keeps its other paint declarations, without a second parse of backgrounds, gradients and shadows here.
    let (env, root_font_size) = s.cascade().environment();
    for raw in s.changed_nodes().skip(pending) {
        let id = LayoutNodeId::from_raw(raw);
        if let Some(computed) = s.computed(raw)
            && tree.layout.node_exists(id)
        {
            let units = css::layout::Units {
                font_size: computed.font_size,
                root_font_size,
                viewport_width: env.width,
                viewport_height: env.height,
            };
            let styles = css::border::resolved_styles(
                computed
                    .resolved
                    .iter()
                    .map(|(name, value)| (s.cascade().str(*name), value.as_str())),
                &units,
            );
            tree.set_border_styles(id, styles);
        }
    }
    s.changed_count() as i32
}

/// Called after native layout and before paint. A nonzero result requests another restyle and layout pass.
pub(crate) fn update_containers(s: &mut Styles, tree: &mut Tree) -> i32 {
    s.update_containers(&TreeHost(tree)) as i32
}

/// Scoped variant for hosts that compute independent or retained roots.
pub(crate) fn update_containers_under(s: &mut Styles, tree: &mut Tree, root: u64) -> i32 {
    s.update_containers_under(&TreeHost(tree), (root != 0).then_some(root)) as i32
}

/// The elements under `root` that the comma-separated `selectors` match, in document order; `None` for
/// selectors that do not read.
pub(crate) fn select(
    s: &mut Styles,
    tree: &mut Tree,
    root: u64,
    selectors: &str,
) -> Option<Vec<u64>> {
    s.select(&TreeHost(tree), root, selectors).ok()
}

/// Every declaration that applies to `node` and where it came from, in cascade order: a record each, split by
/// U+0001, of name, value, sheet id (-1 for the element's own), selector, line, whether `!important` and
/// whether it wins, split by U+0002.
pub(crate) fn explain(s: &mut Styles, tree: &mut Tree, node: u64) -> String {
    let mut out = String::new();
    for o in s.explain(&TreeHost(tree), node) {
        if !out.is_empty() {
            out.push(RECORD);
        }
        let sheet = o.sheet.map_or(-1, |id| id.0 as i64);
        let fields = [
            o.name,
            o.value,
            sheet.to_string(),
            o.selector,
            o.line.to_string(),
            (o.important as u8).to_string(),
            (o.wins as u8).to_string(),
        ];
        out.push_str(&fields.join(&PAIR.to_string()));
    }
    out
}

/// `node`'s style: its resolved declarations (`var()`s replaced), its values (inherited ones and custom
/// properties included), then its font size in pixels, as three records of name-value pairs split by U+0003.
pub(crate) fn style(s: &Styles, node: u64) -> String {
    let Some(c) = s.computed(node) else {
        return String::new();
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
    format!(
        "{}{RECORD}{}{RECORD}{}",
        list(&c.resolved),
        list(&c.values),
        c.font_size
    )
}

#[cfg(feature = "hashlink")]
unsafe fn styles<'a>(h: *mut c_void) -> Option<&'a mut Styles> {
    unsafe { handle_mut::<Styles>(h) }
}

#[cfg(feature = "hashlink")]
mod host {
    use super::*;

    #[unsafe(no_mangle)]
    pub extern "C" fn hl_blinc_css_new() -> *mut c_void {
        into_handle(Styles::new())
    }
    define_prim!(hlp_blinc_css_new, hl_blinc_css_new, "P_Xblinc_css_");

    /// Parses `source` and adds it at position `at`; writes the sheet's id to `id` and returns the diagnostics.
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
        let (source, file) = unsafe { (string_from(source), string_from(file)) };
        let (sheet_id, diagnostics) = add_source(s, &source, &file, at);
        if !id.is_null() {
            unsafe { *id = sheet_id };
        }
        string_to_hl(&diagnostics)
    }
    define_prim!(hlp_blinc_css_add, hl_blinc_css_add, "PXblinc_css_BBiB_B");

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_parse(
        source: *const vbyte,
        file: *const vbyte,
    ) -> *mut c_void {
        let (source, file) = unsafe { (string_from(source), string_from(file)) };
        into_handle(parse_sheet(&source, &file))
    }
    define_prim!(
        hlp_blinc_css_parse,
        hl_blinc_css_parse,
        "PBB_Xblinc_css_sheet_"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_decode(bytes: *const vbyte, len: i32) -> *mut c_void {
        if bytes.is_null() || len < 0 {
            return std::ptr::null_mut();
        }
        let data = unsafe { std::slice::from_raw_parts(bytes, len as usize) };
        match decode_sheet(data) {
            Some(sheet) => into_handle(sheet),
            None => std::ptr::null_mut(),
        }
    }
    define_prim!(
        hlp_blinc_css_decode,
        hl_blinc_css_decode,
        "PBi_Xblinc_css_sheet_"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_sheet_diagnostics(sheet: *mut c_void) -> *mut vbyte {
        let text = unsafe { handle_ref::<css::Stylesheet>(sheet) }
            .map(report)
            .unwrap_or_default();
        string_to_hl(&text)
    }
    define_prim!(
        hlp_blinc_css_sheet_diagnostics,
        hl_blinc_css_sheet_diagnostics,
        "PXblinc_css_sheet__B"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_sheet_imports(sheet: *mut c_void) -> *mut vbyte {
        let text = unsafe { handle_ref::<css::Stylesheet>(sheet) }
            .map(sheet_imports)
            .unwrap_or_default();
        string_to_hl(&text)
    }
    define_prim!(
        hlp_blinc_css_sheet_imports,
        hl_blinc_css_sheet_imports,
        "PXblinc_css_sheet__B"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_sheet_declared(sheet: *mut c_void) -> *mut vbyte {
        let text = unsafe { handle_ref::<css::Stylesheet>(sheet) }
            .map(sheet_declared)
            .unwrap_or_default();
        string_to_hl(&text)
    }
    define_prim!(
        hlp_blinc_css_sheet_declared,
        hl_blinc_css_sheet_declared,
        "PXblinc_css_sheet__B"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_add_parsed(
        h: *mut c_void,
        sheet: *mut c_void,
        at: i32,
    ) -> i32 {
        let (Some(s), Some(sheet)) = (unsafe { styles(h) }, unsafe {
            handle_ref::<css::Stylesheet>(sheet)
        }) else {
            return -1;
        };
        add_parsed(s, sheet, at)
    }
    define_prim!(
        hlp_blinc_css_add_parsed,
        hl_blinc_css_add_parsed,
        "PXblinc_css_Xblinc_css_sheet_i_i"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_keyframes(
        h: *mut c_void,
        name: *const vbyte,
    ) -> *mut vbyte {
        let Some(s) = (unsafe { handle_ref::<Styles>(h) }) else {
            return std::ptr::null_mut();
        };
        match keyframes(s, &unsafe { string_from(name) }) {
            Some(text) => string_to_hl(&text),
            None => std::ptr::null_mut(),
        }
    }
    define_prim!(
        hlp_blinc_css_keyframes,
        hl_blinc_css_keyframes,
        "PXblinc_css_B_B"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_root_variable(
        h: *mut c_void,
        name: *const vbyte,
    ) -> *mut vbyte {
        let Some(s) = (unsafe { handle_ref::<Styles>(h) }) else {
            return std::ptr::null_mut();
        };
        match root_variable(s, &unsafe { string_from(name) }) {
            Some(v) => string_to_hl(&v),
            None => std::ptr::null_mut(),
        }
    }
    define_prim!(
        hlp_blinc_css_root_variable,
        hl_blinc_css_root_variable,
        "PXblinc_css_B_B"
    );

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
        add_compiled(
            s,
            unsafe { std::slice::from_raw_parts(bytes, len as usize) },
            at,
        )
    }
    define_prim!(
        hlp_blinc_css_add_compiled,
        hl_blinc_css_add_compiled,
        "PXblinc_css_Bii_i"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_remove(h: *mut c_void, id: i32) -> i32 {
        unsafe { styles(h) }.map_or(0, |s| remove_sheet(s, id))
    }
    define_prim!(hlp_blinc_css_remove, hl_blinc_css_remove, "PXblinc_css_i_i");

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_set_theme(h: *mut c_void, vars: *const vbyte) {
        if let Some(s) = unsafe { styles(h) } {
            set_theme(s, &unsafe { string_from(vars) });
        }
    }
    define_prim!(
        hlp_blinc_css_set_theme,
        hl_blinc_css_set_theme,
        "PXblinc_css_B_v"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_set_environment(
        h: *mut c_void,
        width: f64,
        height: f64,
        dark: i32,
        root_font_size: f64,
    ) {
        if let Some(s) = unsafe { styles(h) } {
            set_environment(s, width, height, dark != 0, root_font_size);
        }
    }
    define_prim!(
        hlp_blinc_css_set_environment,
        hl_blinc_css_set_environment,
        "PXblinc_css_ddid_v"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_set_element(
        h: *mut c_void,
        node: u64,
        desc: *const vbyte,
    ) {
        if let Some(s) = unsafe { styles(h) } {
            set_element(s, node, &unsafe { string_from(desc) });
        }
    }
    define_prim!(
        hlp_blinc_css_set_element,
        hl_blinc_css_set_element,
        "PXblinc_css_lB_v"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_state_bit(name: *const vbyte) -> i32 {
        state_bit(&unsafe { string_from(name) })
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

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_moved(h: *mut c_void, node: u64) {
        if let Some(s) = unsafe { styles(h) } {
            s.moved(node);
        }
    }
    define_prim!(hlp_blinc_css_moved, hl_blinc_css_moved, "PXblinc_css_l_v");

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

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_restyle(
        h: *mut c_void,
        t: *mut c_void,
        root: u64,
    ) -> i32 {
        let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
            return 0;
        };
        restyle(s, tree, root)
    }
    define_prim!(
        hlp_blinc_css_restyle,
        hl_blinc_css_restyle,
        "PXblinc_css_Xblinc_tree_l_i"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_update_containers(h: *mut c_void, t: *mut c_void) -> i32 {
        let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
            return 0;
        };
        update_containers(s, tree)
    }
    define_prim!(
        hlp_blinc_css_update_containers,
        hl_blinc_css_update_containers,
        "PXblinc_css_Xblinc_tree_i"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_update_containers_under(
        h: *mut c_void,
        t: *mut c_void,
        root: u64,
    ) -> i32 {
        let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
            return 0;
        };
        update_containers_under(s, tree, root)
    }
    define_prim!(
        hlp_blinc_css_update_containers_under,
        hl_blinc_css_update_containers_under,
        "PXblinc_css_Xblinc_tree_l_i"
    );

    /// Writes the nodes whose styles changed, parents first, to `out`, at most `capacity`, and forgets those.
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
        let out = out as *mut u64;
        for (i, raw) in changed.iter().enumerate() {
            unsafe { out.add(i).write_unaligned(*raw) };
        }
        changed.len() as i32
    }
    define_prim!(
        hlp_blinc_css_take_changed,
        hl_blinc_css_take_changed,
        "PXblinc_css_Bi_i"
    );

    /// Writes the states selectors began to test since this was last called, as node and state-bit pairs, at
    /// most `capacity`, and forgets those.
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
        let (nodes, bits) = (nodes as *mut u64, bits as *mut i32);
        for (i, (node, bit)) in watched.iter().enumerate() {
            unsafe {
                nodes.add(i).write_unaligned(*node);
                bits.add(i).write_unaligned(*bit as i32);
            }
        }
        watched.len() as i32
    }
    define_prim!(
        hlp_blinc_css_take_watched,
        hl_blinc_css_take_watched,
        "PXblinc_css_BBi_i"
    );

    /// The elements under `root` that `selectors` match, as 64-bit ids in `out`, at most `capacity`; how many
    /// match (which may be more than it wrote), or -1 for selectors that do not read.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_select(
        h: *mut c_void,
        t: *mut c_void,
        root: u64,
        selectors: *const vbyte,
        out: *mut vbyte,
        capacity: i32,
    ) -> i32 {
        let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
            return -1;
        };
        let Some(found) = select(s, tree, root, &unsafe { string_from(selectors) }) else {
            return -1;
        };
        let out = out as *mut u64;
        for (i, raw) in found.iter().take(capacity.max(0) as usize).enumerate() {
            unsafe { out.add(i).write_unaligned(*raw) };
        }
        found.len() as i32
    }
    define_prim!(
        hlp_blinc_css_select,
        hl_blinc_css_select,
        "PXblinc_css_Xblinc_tree_lBBi_i"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_explain(
        h: *mut c_void,
        t: *mut c_void,
        node: u64,
    ) -> *mut vbyte {
        let (Some(s), Some(tree)) = (unsafe { styles(h) }, unsafe { crate::node::tree(t) }) else {
            return string_to_hl("");
        };
        string_to_hl(&explain(s, tree, node))
    }
    define_prim!(
        hlp_blinc_css_explain,
        hl_blinc_css_explain,
        "PXblinc_css_Xblinc_tree_l_B"
    );

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn hl_blinc_css_style(h: *mut c_void, node: u64) -> *mut vbyte {
        let Some(s) = (unsafe { handle_ref::<Styles>(h) }) else {
            return string_to_hl("");
        };
        string_to_hl(&style(s, node))
    }
    define_prim!(hlp_blinc_css_style, hl_blinc_css_style, "PXblinc_css_l_B");
}

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
        s.restyle_host(&mut TreeHost(&mut tree), root.to_raw());
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
        s.restyle_host(&mut TreeHost(&mut tree), root.to_raw());
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
        s.restyle_host(&mut TreeHost(&mut tree), 0u64);
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

    #[test]
    fn select_finds_elements_in_document_order() {
        let mut tree = Tree::new();
        let root = tree.layout.create_node(Default::default());
        let a = tree.layout.create_node(Default::default());
        let flow = tree.layout.create_node(Default::default());
        let b = tree.layout.create_node(Default::default());
        let inner = tree.layout.create_node(Default::default());
        for n in [a, flow, b] {
            tree.layout.add_child(root, n);
        }
        tree.layout.add_child(a, inner);
        let mut s = Styles::new();
        for (n, classes) in [
            (root, &["list"][..]),
            (a, &["row"]),
            (b, &["row"]),
            (inner, &["row", "deep"]),
        ] {
            let e = element(&mut s, &["div"], classes);
            s.set_element(n.to_raw(), e);
        }
        let mut e = element(&mut s, &["div"], &["row"]);
        e.anonymous = true;
        s.set_element(flow.to_raw(), e);
        let host = TreeHost(&mut tree);
        let raw = |v: &[LayoutNodeId]| v.iter().map(|n| n.to_raw()).collect::<Vec<_>>();
        assert_eq!(
            s.select(&host, root.to_raw(), ".row").unwrap(),
            raw(&[a, inner, b])
        );
        assert_eq!(
            s.select(&host, root.to_raw(), ".list, .deep").unwrap(),
            raw(&[root, inner])
        );
        assert_eq!(
            s.select(&host, root.to_raw(), ".row:first-child").unwrap(),
            raw(&[a, inner])
        );
        assert!(s.select(&host, root.to_raw(), ".row >").is_err());
    }

    #[test]
    fn keyframes_and_root_variables_come_from_the_latest_sheet() {
        let mut s = Styles::new();
        let mut load = |_: &str, _: Option<&str>| None;
        s.cascade_mut().push(css::parse(
            ":root { --gap: 4px; --tone: red } @keyframes pulse { from { opacity: 1 } to { opacity: 0 } }",
            None,
            &mut load,
        ));
        s.cascade_mut().push(css::parse(
            ":root { --gap: 8px } @keyframes pulse { 0%, 50% { opacity: 0.5 } }",
            None,
            &mut load,
        ));
        assert_eq!(s.cascade().root_variable("gap"), Some("8px"));
        assert_eq!(s.cascade().root_variable("tone"), Some("red"));
        assert_eq!(s.cascade().root_variable("none"), None);
        let (sheet, k) = s.cascade().keyframes("pulse").unwrap();
        let frames = sheet.keyframe_list(k);
        assert_eq!(frames.len(), 1);
        assert_eq!(sheet.keyframe_offsets(&frames[0]), [0.0, 0.5]);
        assert!(s.cascade().keyframes("none").is_none());
    }
}
