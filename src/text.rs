//! Glyphs for text nodes: shaped by Blinc's text engine and rasterized into
//! its glyph atlases, which ashui uploads and samples in its own text shader.
//!
//! Glyphs are rasterized at the size they cover on screen, the font size
//! times the display scale times the node's transform scale, so text stays
//! sharp when zoomed. Scales are bucketed at twelve steps per doubling, so an
//! animated zoom reuses glyphs instead of rasterizing every frame.

use blinc_layout::div::{GenericFont as LayoutGeneric, TextAlign};
use blinc_layout::tree::TextMeasureContext;
use blinc_text::{
    GenericFont, LayoutOptions, LineBreakMode, PreparedText, SubpixelX, TextAlignment, TextAnchor,
    TextError, TextRenderer, global_font_registry,
};
#[cfg(feature = "hashlink")]
use hl_abi::{define_prim, vbyte};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

static RENDERER: LazyLock<Mutex<TextRenderer>> =
    LazyLock::new(|| Mutex::new(TextRenderer::with_shared_registry(global_font_registry())));

pub fn renderer() -> MutexGuard<'static, TextRenderer> {
    RENDERER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Install font-backed measurement once for every adapter sharing this engine.
pub fn initialize() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| blinc_layout::init_text_measurer_with_registry(global_font_registry()));
}

/// Loads the face `context` is set in, if the renderer has not yet: layout
/// measures only with loaded faces, and estimates the rest.
pub fn ensure_face(context: &TextMeasureContext) {
    let generic = generic(context.generic_font);
    let registry = global_font_registry();
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    let name = context.font_name.as_deref();
    if registry
        .get_for_render_with_style(name, generic, context.font_weight, context.italic)
        .is_none()
    {
        let _ =
            registry.load_with_fallback_styled(name, generic, context.font_weight, context.italic);
    }
}

/// The faces `system-ui` names, by platform, as browsers resolve it: on
/// macOS SF Pro, a variable face Blinc sets each weight of on its `wght`
/// axis, then Helvetica Neue where it is missing.
#[cfg(target_os = "macos")]
const SYSTEM_UI: &[&str] = &[".SF NS", "Helvetica Neue"];
#[cfg(target_os = "windows")]
const SYSTEM_UI: &[&str] = &["Segoe UI Variable", "Segoe UI"];
/// A page has no platform face: the first of these the program has registered, so a Helvetica
/// of its own takes the place of the Arimo it is built with.
#[cfg(target_family = "wasm")]
const SYSTEM_UI: &[&str] = &["Helvetica Neue", "Helvetica", "Arial", "Arimo"];
#[cfg(not(any(target_os = "macos", target_os = "windows", target_family = "wasm")))]
const SYSTEM_UI: &[&str] = &[];

/// A CSS font stack resolved as a browser does: the first family installed,
/// or the first generic keyword (`monospace`, `serif`, `sans-serif`,
/// `system-ui` and kin) as Blinc's generic face, `system-ui` as the
/// platform's UI face where it is installed. `ui-monospace`,
/// `ui-serif` and `ui-sans-serif` name the platform's own face, which the
/// families after them usually list, so they are used only if none of
/// those is installed. A stack with none is the system face.
pub fn resolve_family(stack: &str) -> (Option<String>, LayoutGeneric) {
    let registry = global_font_registry();
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    let mut platform = None;
    for name in stack.split(',') {
        let name = name.trim().trim_matches(|c| c == '"' || c == '\'');
        let generic = match name.to_ascii_lowercase().as_str() {
            "" => continue,
            "ui-monospace" => {
                platform.get_or_insert(LayoutGeneric::Monospace);
                continue;
            }
            "ui-serif" => {
                platform.get_or_insert(LayoutGeneric::Serif);
                continue;
            }
            "ui-sans-serif" => {
                platform.get_or_insert(LayoutGeneric::SansSerif);
                continue;
            }
            "monospace" => Some(LayoutGeneric::Monospace),
            "serif" => Some(LayoutGeneric::Serif),
            "sans-serif" => Some(LayoutGeneric::SansSerif),
            "system-ui" | "-apple-system" | "blinkmacsystemfont" => Some(LayoutGeneric::System),
            _ => None,
        };
        if let Some(g) = generic {
            // The platform's UI face for system-ui, as a browser's: its metrics, not a generic sans-serif's, centre capitals in their line.
            if g == LayoutGeneric::System && platform.is_none() {
                drop(registry);
                return (system_ui(), LayoutGeneric::System);
            }
            // A page has no generic sans-serif; its one face is the system UI face.
            if cfg!(target_family = "wasm") && g == LayoutGeneric::SansSerif && platform.is_none() {
                drop(registry);
                if let Some(face) = system_ui() {
                    return (Some(face), LayoutGeneric::System);
                }
                return (None, g);
            }
            return (None, platform.unwrap_or(g));
        }
        if registry.has_font(name) {
            return (Some(name.to_string()), LayoutGeneric::System);
        }
    }
    match platform {
        Some(p) => (None, p),
        None => {
            drop(registry);
            (system_ui(), LayoutGeneric::System)
        }
    }
}

/// The installed face of `SYSTEM_UI`, looked up once: what text with no family of its own is set in.
pub fn system_ui() -> Option<String> {
    static FACE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FACE.get_or_init(|| {
        let registry = global_font_registry();
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        SYSTEM_UI
            .iter()
            .find(|f| registry.has_font(f))
            .map(|f| f.to_string())
    })
    .clone()
}

fn generic(g: LayoutGeneric) -> GenericFont {
    match g {
        LayoutGeneric::Monospace => GenericFont::Monospace,
        LayoutGeneric::Serif => GenericFont::Serif,
        LayoutGeneric::SansSerif => GenericFont::SansSerif,
        _ => GenericFont::System,
    }
}

/// How many times the glyph atlases have been cleared to make room. Every atlas rectangle handed out before a clear
/// is stale after it, so a caller that keeps rectangles compares this with the value it saw.
static ATLAS_GENERATION: AtomicI32 = AtomicI32::new(0);

/// Bits of the `flags` argument of `hl_blinc_text_prepare` and `hl_blinc_text_advances`; none set is the default.
/// `HINT`: round each character's advance to a whole pixel, kerning excepted, as a hinted layout does.
#[cfg(feature = "hashlink")]
const HINT: i32 = 1;

/// Records that the glyph atlases were cleared; every clear in this library goes through it.
pub(crate) fn note_atlas_cleared() {
    ATLAS_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The offsets within a pixel a glyph may be rasterized at: thirds.
pub const SUBPIXEL_PHASES: std::num::NonZeroU8 = std::num::NonZeroU8::new(3).unwrap();

/// Steps per doubling of scale that glyphs are rasterized at.
const STEPS_PER_OCTAVE: f32 = 12.0;

/// The largest size a glyph is rasterized at; beyond it glyphs are magnified.
const MAX_RASTER_SIZE: f32 = 256.0;

/// The scale `font_size` is rasterized at for `on_screen`, the device pixels
/// per layout unit: `on_screen` on its bucket, capped by `MAX_RASTER_SIZE`.
pub fn raster_scale(font_size: f32, on_screen: f32) -> f32 {
    if on_screen.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
        || font_size.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
    {
        return 1.0;
    }
    let bucket = ((on_screen.log2() * STEPS_PER_OCTAVE).round() / STEPS_PER_OCTAVE).exp2();
    bucket.min(MAX_RASTER_SIZE / font_size).max(1.0 / 64.0)
}

/// `context`'s text laid out in a box `width` wide, at `scale` times its size:
/// glyph bounds and atlas rects in raster pixels. With `subpixel`, each glyph
/// is on the whole pixel Blinc chose for it, from the run's origin snapped down,
/// its fraction rasterized into it. `Err(AtlasFull)` when the atlases cannot
/// take its glyphs.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    renderer: &mut TextRenderer,
    context: &TextMeasureContext,
    width: f32,
    align: Option<TextAlign>,
    letter_spacing: f32,
    color: [f32; 4],
    scale: f32,
    subpixel: Option<SubpixelX>,
) -> Result<PreparedText, TextError> {
    let options = LayoutOptions {
        // Layout rounds sizes to whole pixels, which can leave the box up to
        // half a pixel narrower than the text measured for it; the allowance
        // keeps such a line from wrapping.
        max_width: Some((width + 1.0) * scale),
        alignment: match align {
            Some(TextAlign::Center) => TextAlignment::Center,
            Some(TextAlign::Right) => TextAlignment::Right,
            _ => TextAlignment::Left,
        },
        anchor: TextAnchor::Top,
        line_break: if context.wrap {
            LineBreakMode::Word
        } else {
            LineBreakMode::None
        },
        line_height: context.line_height,
        letter_spacing: letter_spacing * scale,
    };
    let generic = generic(context.generic_font);
    renderer.prepare_text_subpixel(
        &context.content,
        context.font_size * scale,
        color,
        &options,
        context.font_name.as_deref(),
        generic,
        context.font_weight,
        context.italic,
        subpixel,
    )
}

/// How far below the top of its line box a line's glyphs start, as CSS
/// places them: half of what the line box has beyond the font's ascender
/// and descender, at `font_size`. Blinc's layout puts the first baseline at
/// the ascender, leaving all of that space below the text.
pub fn half_leading(context: &TextMeasureContext, font_size: f32) -> f32 {
    let Some(m) = face_metrics(context) else {
        return 0.0;
    };
    let line = font_size * context.line_height;
    (line - (m.ascender_px(font_size) - m.descender_px(font_size))) / 2.0
}

/// The metrics of the face `context` is drawn in.
fn face_metrics(context: &TextMeasureContext) -> Option<blinc_text::FontMetrics> {
    let registry = global_font_registry();
    let registry = registry.lock().ok()?;
    let generic = match context.generic_font {
        LayoutGeneric::Monospace => GenericFont::Monospace,
        LayoutGeneric::Serif => GenericFont::Serif,
        LayoutGeneric::SansSerif => GenericFont::SansSerif,
        _ => GenericFont::System,
    };
    registry
        .get_for_render_with_style(
            context.font_name.as_deref(),
            generic,
            context.font_weight,
            context.italic,
        )
        .map(|font| *font.metrics())
}

/// Where a font puts its text decorations at `font_size`, in pixels from the baseline: the
/// underline's top below it, its thickness, the strike-through's middle above it, the top of
/// the em above it. A font without the figures gets proportions that look like most.
#[derive(Clone, Copy, Debug)]
pub struct DecorationMetrics {
    pub underline: f32,
    pub thickness: f32,
    pub strike: f32,
    pub ascender: f32,
}

pub fn decoration_metrics(context: &TextMeasureContext, font_size: f32) -> DecorationMetrics {
    let Some(m) = face_metrics(context) else {
        return DecorationMetrics {
            underline: font_size * 0.1,
            thickness: font_size / 14.0,
            strike: font_size * 0.3,
            ascender: font_size * 0.8,
        };
    };
    DecorationMetrics {
        underline: m
            .underline_position
            .map_or(font_size * 0.1, |p| -m.scale(p, font_size)),
        thickness: m
            .underline_thickness
            .map_or(font_size / 14.0, |t| m.scale(t, font_size)),
        strike: m
            .x_height
            .map_or(font_size * 0.3, |x| m.scale(x, font_size) / 2.0),
        ascender: m.ascender_px(font_size),
    }
}

// ============================================================================
// ATLASES
// ============================================================================

#[cfg(feature = "hashlink")]
type AtlasChange = (i32, (u32, u32, u32, u32), (u32, u32));

/// The changes an atlas has had, newest last, each with the region it wrote
/// and the atlas's size then; enough of them for a caller a few frames behind.
#[cfg(feature = "hashlink")]
struct AtlasHistory {
    revision: i32,
    changes: std::collections::VecDeque<AtlasChange>,
}

/// How many changes an atlas remembers; a caller further behind takes it whole.
#[cfg(feature = "hashlink")]
const ATLAS_HISTORY: usize = 32;

/// The coverage atlas's history, then the colour one's.
#[cfg(feature = "hashlink")]
static HISTORIES: Mutex<[AtlasHistory; 2]> = Mutex::new([
    AtlasHistory {
        revision: 0,
        changes: std::collections::VecDeque::new(),
    },
    AtlasHistory {
        revision: 0,
        changes: std::collections::VecDeque::new(),
    },
]);

/// The coverage atlas (`color` 0, one byte a pixel) or the colour-glyph atlas
/// (`color` 1, RGBA). Each change bumps its revision. When the revision is
/// not `seen`, writes seven i32s to `info`: the atlas's width, height and
/// revision, then the region `x`, `y`, `width` and `height` changed since
/// `seen`, the whole atlas when `seen` is 0, too old, or older than its last
/// growth. Returns the region's size in bytes, copying its rows, packed, into
/// `out` when they fit in `capacity`. Returns 0 when the caller has seen this
/// revision.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_text_atlas_take(
    color: i32,
    seen: i32,
    out: *mut vbyte,
    capacity: i32,
    info: *mut vbyte,
) -> i32 {
    let mut r = renderer();
    let color = color != 0;
    let mut histories = HISTORIES.lock().unwrap_or_else(|e| e.into_inner());
    let history = &mut histories[color as usize];
    let (dirty, rect, dims) = if color {
        (
            r.color_atlas_is_dirty(),
            r.color_atlas_dirty_rect(),
            r.color_atlas_dimensions(),
        )
    } else {
        (
            r.atlas_is_dirty(),
            r.atlas_dirty_rect(),
            r.atlas_dimensions(),
        )
    };
    if dirty {
        if color {
            r.mark_color_atlas_clean()
        } else {
            r.mark_atlas_clean()
        }
        history.revision += 1;
        history.changes.push_back((
            history.revision,
            rect.unwrap_or((0, 0, dims.0, dims.1)),
            dims,
        ));
        if history.changes.len() > ATLAS_HISTORY {
            history.changes.pop_front();
        }
    }
    let revision = history.revision;
    if revision == seen {
        return 0;
    }
    // The union of what changed after `seen`, when every change since is remembered and the atlas kept its size.
    let since: Vec<_> = history.changes.iter().filter(|c| c.0 > seen).collect();
    let known = seen > 0
        && since.first().is_some_and(|c| c.0 == seen + 1)
        && since.iter().all(|c| c.2 == dims);
    let (x, y, w, h) = if known {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for &&(_, (x, y, w, h), _) in &since {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + w);
            y1 = y1.max(y + h);
        }
        (x0, y0, x1.min(dims.0) - x0, y1.min(dims.1) - y0)
    } else {
        (0, 0, dims.0, dims.1)
    };
    if !info.is_null() {
        let info = info as *mut i32;
        for (i, v) in [
            dims.0 as i32,
            dims.1 as i32,
            revision,
            x as i32,
            y as i32,
            w as i32,
            h as i32,
        ]
        .into_iter()
        .enumerate()
        {
            unsafe { info.add(i).write_unaligned(v) };
        }
    }
    let bpp = if color { 4 } else { 1 };
    let row = w as usize * bpp;
    let size = row * h as usize;
    if !out.is_null() && size <= capacity.max(0) as usize {
        let pixels = if color {
            r.color_atlas_pixels()
        } else {
            r.atlas_pixels()
        };
        let stride = dims.0 as usize * bpp;
        for j in 0..h as usize {
            let from = (y as usize + j) * stride + x as usize * bpp;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    pixels[from..from + row].as_ptr(),
                    out.add(j * row),
                    row,
                )
            };
        }
    }
    size as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_text_atlas_take,
    hl_blinc_text_atlas_take,
    "PiiBiB_i"
);

// ============================================================================
// CARETS
// ============================================================================

/// A place a caret can stand: before the character at `index` (a UTF-16
/// index, as JavaScript and Haxe count), at `x` on line `line`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub index: u32,
    pub x: f32,
    pub line: u32,
}

/// One laid-out line: the UTF-16 range it shows and its width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredLine {
    pub start: u32,
    pub end: u32,
    pub width: f32,
}

/// Text laid out as the renderer lays it out, without rasterizing it.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    /// Every character boundary, in order; a blank line still has one.
    pub carets: Vec<Caret>,
    pub lines: Vec<MeasuredLine>,
    /// The widest line.
    pub width: f32,
    pub line_height: f32,
    /// The font's ascender, and its descender (below the baseline, so negative).
    pub ascender: f32,
    pub descender: f32,
}

/// `text` set in `context`'s font, size, weight and line height, with
/// `letter_spacing`, as the renderer draws it. With `wrap_width`, lines wrap
/// at that width (with the renderer's allowance for rounding); a line break
/// always ends a line. `None` when no face can be loaded for the font.
pub fn measure(
    context: &TextMeasureContext,
    text: &str,
    letter_spacing: f32,
    wrap_width: Option<f32>,
) -> Option<Measured> {
    // The renderer wraps at the laid-out width plus its allowance for rounding.
    let max_width = wrap_width.filter(|w| *w > 0.0).map(|w| w + 1.0);
    let generic = generic(context.generic_font);
    // The face, and the fallback faces its text needs, as Blinc lays it out to draw it.
    let (font, fallbacks) = {
        let registry = global_font_registry();
        let mut registry = registry.lock().ok()?;
        let font = match registry.get_for_render_with_style(
            context.font_name.as_deref(),
            generic,
            context.font_weight,
            context.italic,
        ) {
            Some(font) => font,
            None => registry.load_generic(generic).ok()?,
        };
        let fallbacks = blinc_text::FallbackFaces::resolve(&mut registry, &font, text);
        (font, fallbacks)
    };
    let size = context.font_size;
    let options = LayoutOptions {
        max_width,
        alignment: TextAlignment::Left,
        anchor: TextAnchor::Top,
        line_break: if max_width.is_some() {
            LineBreakMode::Word
        } else {
            LineBreakMode::None
        },
        line_height: context.line_height,
        letter_spacing,
    };
    // UTF-16 index of every byte offset that starts a character.
    let mut utf16 = vec![0u32; text.len() + 1];
    let mut units = 0u32;
    for (byte, ch) in text.char_indices() {
        utf16[byte] = units;
        units += ch.len_utf16() as u32;
    }
    utf16[text.len()] = units;
    // Each paragraph between line breaks is laid out on its own, as the
    // renderer breaks lines at them; an empty one is still a line.
    let engine = blinc_text::TextLayoutEngine::new();
    let mut out = Measured {
        line_height: size * context.line_height,
        ..Default::default()
    };
    let mut line = 0u32;
    let mut base = 0usize;
    for paragraph in text.split('\n') {
        let layout = engine.layout_with_fallbacks(paragraph, &font, size, &options, &fallbacks);
        if layout.lines.is_empty() {
            out.carets.push(Caret {
                index: utf16[base],
                x: 0.0,
                line,
            });
            out.lines.push(MeasuredLine {
                start: utf16[base],
                end: utf16[base],
                width: 0.0,
            });
            line += 1;
        }
        let ranges = layout.line_byte_ranges(paragraph.len());
        for (i, l) in layout.lines.iter().enumerate() {
            for g in &l.glyphs {
                let index = utf16[(base + g.byte_offset).min(text.len())];
                if out.carets.last().is_none_or(|c| c.index != index) {
                    out.carets.push(Caret {
                        index,
                        x: g.x,
                        line,
                    });
                }
            }
            // The paragraph's end: before its line break, or the text's end.
            if i + 1 == layout.lines.len() {
                let end = utf16[base + paragraph.len()];
                if out.carets.last().is_none_or(|c| c.index != end) {
                    out.carets.push(Caret {
                        index: end,
                        x: l.width,
                        line,
                    });
                }
            }
            let range = ranges.get(i).cloned().unwrap_or(0..paragraph.len());
            out.lines.push(MeasuredLine {
                start: utf16[(base + range.start).min(text.len())],
                end: utf16[(base + range.end).min(text.len())],
                width: l.width,
            });
            out.width = out.width.max(l.width);
            line += 1;
        }
        base += paragraph.len() + 1;
    }
    let m = font.metrics();
    out.ascender = m.ascender_px(size);
    out.descender = m.descender_px(size);
    Some(out)
}

/// Where a caret can stand in `text`, set in text node `node`'s font and
/// letter spacing at `font_size` (the node's own when 0): for each character
/// boundary, its index in the string as UTF-16 (Haxe's indexing), its x and
/// its line, as three f32s in `out`, at most `capacity` of them. With
/// `wrap_width` above 0, lines wrap at that width, the width the node is laid
/// out at, with the allowance the renderer gives it.
/// A line break ends a line; a blank line still has a stop. Writes the line
/// height, the number of lines, and the font's ascender and descender (below
/// the baseline, so negative) at that size as four f32s to `info`. Returns how many
/// stops there are; 0 when the node is not text or its font is not loaded.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_text_carets(
    h: *mut std::ffi::c_void,
    node: u64,
    text: *const vbyte,
    font_size: f32,
    wrap_width: f32,
    out: *mut vbyte,
    capacity: i32,
    info: *mut vbyte,
) -> i32 {
    let Some(tree) = (unsafe { crate::node::tree(h) }) else {
        return 0;
    };
    let id = blinc_layout::tree::LayoutNodeId::from_raw(node);
    let Some(context) = tree.layout.text_context(id) else {
        return 0;
    };
    let text = unsafe { crate::hl::string_from(text) };
    let letter_spacing = tree
        .props
        .get(&id)
        .and_then(|p| p.letter_spacing)
        .unwrap_or(0.0);
    let mut context = context.clone();
    if font_size > 0.0 {
        context.font_size = font_size;
    }
    let Some(measured) = measure(&context, &text, letter_spacing, Some(wrap_width)) else {
        return 0;
    };
    if !info.is_null() {
        let info = info as *mut f32;
        unsafe {
            info.write_unaligned(measured.line_height);
            info.add(1)
                .write_unaligned(measured.lines.len().max(1) as f32);
            info.add(2).write_unaligned(measured.ascender);
            info.add(3).write_unaligned(measured.descender);
        }
    }
    if !out.is_null() {
        let out = out as *mut f32;
        for (i, c) in measured
            .carets
            .iter()
            .take(capacity.max(0) as usize)
            .enumerate()
        {
            for (j, v) in [c.index as f32, c.x, c.line as f32].into_iter().enumerate() {
                unsafe { out.add(i * 3 + j).write_unaligned(v) };
            }
        }
    }
    measured.carets.len() as i32
}
#[cfg(feature = "hashlink")]
define_prim!(
    hlp_blinc_text_carets,
    hl_blinc_text_carets,
    "PXblinc_tree_lBffBiB_i"
);

#[cfg(test)]
mod tests {
    use super::*;
    use blinc_layout::text_measure::{TextLayoutOptions, measure_text_with_options};

    /// Layout measures text as wide as it is drawn, so a word laid out on one
    /// line is not broken when drawn: regular and bold alike.
    #[test]
    fn layout_measures_text_as_drawn() {
        blinc_layout::init_text_measurer_with_registry(renderer().font_registry());
        let options = LayoutOptions {
            max_width: None,
            alignment: TextAlignment::Left,
            anchor: TextAnchor::Top,
            line_break: LineBreakMode::None,
            line_height: 1.2,
            letter_spacing: 0.0,
        };
        for weight in [400u16, 700] {
            ensure_face(&TextMeasureContext {
                content: "Card".into(),
                font_size: 12.0,
                line_height: 1.2,
                letter_spacing: 0.0,
                wrap: false,
                font_name: None,
                generic_font: LayoutGeneric::System,
                font_weight: weight,
                italic: false,
            });
            let mut measure = TextLayoutOptions::new();
            measure.font_weight = weight;
            let measured = measure_text_with_options("Card", 12.0, &measure).width;
            let drawn = renderer()
                .prepare_text_with_style(
                    "Card",
                    12.0,
                    [1.0; 4],
                    &options,
                    None,
                    GenericFont::System,
                    weight,
                    false,
                )
                .unwrap()
                .width;
            assert!(
                (measured - drawn).abs() < 0.01,
                "weight {weight}: measured {measured}, drawn {drawn}"
            );
        }
    }
    #[test]
    fn measure_reports_lines_carets_and_metrics() {
        let context = TextMeasureContext {
            content: String::new(),
            font_size: 16.0,
            line_height: 1.5,
            letter_spacing: 0.0,
            wrap: true,
            font_name: None,
            generic_font: LayoutGeneric::System,
            font_weight: 400,
            italic: false,
        };
        ensure_face(&context);
        let text = "a\u{1F600}b\n\nwide words wrap here";
        let one = measure(&context, text, 0.0, None).unwrap();
        // Three paragraphs, the middle one empty; carets at UTF-16 indices.
        assert_eq!(one.lines.len(), 3);
        assert_eq!(one.lines[1].start, one.lines[1].end);
        assert!(
            one.carets.iter().any(|c| c.index == 3),
            "after the surrogate pair"
        );
        assert!(!one.carets.iter().any(|c| c.index == 2), "not inside it");
        assert_eq!(one.line_height, 24.0);
        assert!(one.ascender > 0.0 && one.descender < 0.0);
        let wrapped = measure(&context, text, 0.0, Some(one.lines[2].width / 2.0)).unwrap();
        assert!(wrapped.lines.len() > 3);
        assert!(wrapped.carets.windows(2).all(|w| w[0].index < w[1].index));
        // Letter spacing widens the line.
        let spaced = measure(&context, text, 2.0, None).unwrap();
        assert!(spaced.width > one.width);
    }
    #[test]
    fn prepare_glyphs_rasterizes_into_atlas_and_reports_metrics() {
        let mut out = [0.0f32; 256];
        let mut info = [0.0f32; 4];
        let style = [0.0, 400.0, 0.0, 16.0, 0.0, 1.2];
        let n = unsafe {
            prepare_glyphs(
                "Hello\nWorld",
                None,
                style,
                out.as_mut_ptr() as *mut u8,
                out.len() as i32,
                info.as_mut_ptr() as *mut u8,
            )
        };
        assert!(n >= 90 && n % 9 == 0);
        assert!(info[0] > 0.0 && info[1] > 0.0 && info[2] > 0.0 && info[3] > 0.0);
        // Second line's glyphs sit below the first line.
        assert!(out[5 * 9 + 1] > out[1]);
    }
    #[test]
    fn composed_lines_match_whole_lines() {
        let options = LayoutOptions {
            max_width: None,
            alignment: TextAlignment::Left,
            anchor: TextAnchor::Top,
            line_break: LineBreakMode::None,
            line_height: 1.0,
            letter_spacing: 0.0,
        };
        let run = RunStyle {
            key: StyleKey {
                name: None,
                generic: GenericFont::System,
                weight: 400,
                italic: false,
                size: 16.0f32.to_bits(),
                spacing: 0.0f32.to_bits(),
                hint: false,
            },
            name: None,
            generic: GenericFont::System,
            weight: 400,
            italic: false,
            size: 16.0,
            options: &options,
            hint: false,
        };
        let mut r = renderer();
        let mut style = StyleCache::new();
        let mut units = 0usize;
        for text in ["Hello world", "AVATAR To Wa Ty", "caf\u{e9} \u{4f60}\u{597d}", "fi fl office", "  spaced  out "] {
            let (mut composed, mut whole) = (Vec::new(), Vec::new());
            let composed_width = {
                let mut composer = Composer { r: &mut r, run: &run, style: &mut style, units: &mut units };
                compose_paragraph(&mut composer, text, 0.0, &mut composed, None)
            };
            let whole_width = whole_paragraph(&mut r, &run, text, 0.0, &mut whole);
            assert_eq!(composed.len(), whole.len(), "glyph count of {text:?}");
            assert!((composed_width - whole_width).abs() < 0.5, "width of {text:?}: {composed_width} against {whole_width}");
            for (c, w) in composed.chunks(9).zip(whole.chunks(9)) {
                assert!((c[0] - w[0]).abs() <= 1.0, "glyph x in {text:?}: {} against {}", c[0], w[0]);
                assert_eq!((c[2], c[3]), (w[2], w[3]), "glyph size in {text:?}");
            }
        }
        assert!(units > 0);
    }
    #[test]
    fn hinted_advances_are_whole_pixels_and_do_not_mix_with_exact_ones() {
        let style = [0.0, 400.0, 0.0, 14.0, 0.0, 1.2];
        let lay = |text: &str, hint: bool| {
            let mut advances = Vec::new();
            let mut info = [0.0f32; 4];
            unsafe {
                prepare_glyphs_with(text, None, style, hint, std::ptr::null_mut(), 0, info.as_mut_ptr() as *mut u8, Some(&mut advances));
            }
            (advances, info[0])
        };
        let (exact, exact_width) = lay("mmmm", false);
        let (hinted, hinted_width) = lay("mmmm", true);
        assert!(hinted.iter().all(|a| (a - a.round()).abs() < 0.001), "hinted advances {hinted:?} are whole pixels");
        assert!((hinted_width - 4.0 * exact[0].round()).abs() < 0.01, "hinted width {hinted_width} from exact {exact:?}");
        // The two modes keep apart in the cache: asking for exact again gives the exact result.
        let (_, again) = lay("mmmm", false);
        assert!((again - exact_width).abs() < 0.001, "exact width changed from {exact_width} to {again}");
        // A space is a unit like any other.
        let (spaced, _) = lay("m m", true);
        assert!(spaced.iter().all(|a| (a - a.round()).abs() < 0.001) && spaced[1] > 0.0);
    }
    #[test]
    fn advances_cover_every_character_and_add_up_to_the_width() {
        let style = [0.0, 400.0, 0.0, 16.0, 0.0, 1.2];
        for text in ["Hello world", "  two  spaces ", "AVATAR To", "fi office", "caf\u{e9} \u{4f60}\u{597d}", "one\ntwo"] {
            let mut advances = Vec::new();
            let mut info = [0.0f32; 4];
            unsafe {
                prepare_glyphs_with(text, None, style, false, std::ptr::null_mut(), 0, info.as_mut_ptr() as *mut u8, Some(&mut advances));
            }
            assert_eq!(advances.len(), text.chars().count(), "one advance per character of {text:?}");
            let widest = text
                .split('\n')
                .scan(0, |start, line| {
                    let count = line.chars().count();
                    let sum: f32 = advances[*start..*start + count].iter().sum();
                    *start += count + 1;
                    Some(sum)
                })
                .fold(0.0f32, f32::max);
            assert!((widest - info[0]).abs() < 0.01, "advances of {text:?} add up to {widest}, the width is {}", info[0]);
            for (c, a) in text.chars().zip(&advances) {
                if c == ' ' {
                    assert!(*a > 0.0, "a space has an advance in {text:?}");
                }
            }
        }
    }
}

// ============================================================================
// OUTLINES
// ============================================================================

/// Path commands for a glyph outline, in pixels, the baseline at y 0 and y
/// growing down: 0 move (x, y), 1 line (x, y), 2 quadratic (cx, cy, x, y),
/// 3 cubic (c1x, c1y, c2x, c2y, x, y), 4 close.
#[cfg(feature = "hashlink")]
struct Outline {
    out: Vec<f32>,
    x: f32,
    y: f32,
}

#[cfg(feature = "hashlink")]
impl Outline {
    /// A glyph's outline, in pixels with y up, placed with its origin at
    /// `(x, y)` in the canvas's coordinates, y down.
    fn add(&mut self, path: &swash::scale::outline::Outline) {
        use swash::zeno::{Command, PathData};
        let (x, y) = (self.x, self.y);
        for command in path.path().commands() {
            match command {
                Command::MoveTo(p) => self.out.extend([0.0, x + p.x, y - p.y]),
                Command::LineTo(p) => self.out.extend([1.0, x + p.x, y - p.y]),
                Command::QuadTo(c, p) => self.out.extend([2.0, x + c.x, y - c.y, x + p.x, y - p.y]),
                Command::CurveTo(c1, c2, p) => self.out.extend([
                    3.0,
                    x + c1.x,
                    y - c1.y,
                    x + c2.x,
                    y - c2.y,
                    x + p.x,
                    y - p.y,
                ]),
                Command::Close => self.out.push(4.0),
            }
        }
    }
}

/// `text` laid out in a face of `font_name` (else the generic family), as
/// `style` sets it, six f32s: generic family (0 system, 1 monospace, 2
/// serif, 3 sans-serif), weight, italic (0 or 1), size in pixels, letter
/// spacing and line height; as path commands (see `Outline`) of every
/// glyph: the first line's baseline at y 0, each line after it
/// `line_height` times the font size below, or the face's own line where it
/// is 0. Writes the widest line's width, the face's
/// ascent and descent (positive, below the baseline) and its line height,
/// in pixels, as four f32s to `info`. Returns the number of f32s, copying
/// them to `out` when they fit in `capacity`; 0 when no face is found.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn hl_blinc_text_outline(
    text: *const vbyte,
    font_name: *const vbyte,
    style: *const vbyte,
    out: *mut vbyte,
    capacity: i32,
    info: *mut vbyte,
) -> i32 {
    if style.is_null() {
        return 0;
    }
    let s = |i: usize| unsafe { (style as *const f32).add(i).read_unaligned() };
    let (generic, weight, italic, size, letter_spacing, line_height) =
        (s(0) as i32, s(1) as i32, s(2) != 0.0, s(3), s(4), s(5));
    let text = unsafe { crate::hl::string_from(text) };
    let name = unsafe { crate::hl::opt_string_from(font_name) };
    let generic = match generic {
        1 => GenericFont::Monospace,
        2 => GenericFont::Serif,
        3 => GenericFont::SansSerif,
        _ => GenericFont::System,
    };
    // Resolved before the registry is locked: finding the system face locks it too.
    let name = name.or_else(|| {
        if generic == GenericFont::System {
            system_ui()
        } else {
            None
        }
    });
    // From the renderer's registry, loaded as text nodes' faces are, so a weight not yet used is found.
    let (font, fallbacks) = {
        let registry = global_font_registry();
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        let weight = weight.clamp(1, 1000) as u16;
        let font =
            match registry.get_for_render_with_style(name.as_deref(), generic, weight, italic) {
                Some(font) => font,
                None => match registry.load_with_fallback_styled(
                    name.as_deref(),
                    generic,
                    weight,
                    italic,
                ) {
                    Ok(font) => font,
                    Err(_) => return 0,
                },
            };
        // The faces glyphs the primary lacks are drawn from: symbols, emoji, CJK.
        let fallbacks = blinc_text::FallbackFaces::resolve(&mut registry, &font, &text);
        (font, fallbacks)
    };
    // Scaled by swash, as Blinc rasterizes text, each face at the weight Blinc shapes it at.
    let mut context = swash::scale::ScaleContext::new();
    let metrics = font.metrics();
    let ascent = metrics.ascender_px(size);
    let descent = -metrics.descender_px(size);
    let natural = metrics.line_height_px(size);
    let options = LayoutOptions {
        max_width: None,
        alignment: TextAlignment::Left,
        anchor: TextAnchor::Top,
        line_break: LineBreakMode::None,
        line_height: 1.0,
        letter_spacing,
    };
    let engine = blinc_text::TextLayoutEngine::new();
    let mut outline = Outline {
        out: Vec::new(),
        x: 0.0,
        y: 0.0,
    };
    let mut widest = 0.0f32;
    for (row, paragraph) in text.split('\n').enumerate() {
        let layout = engine.layout_with_fallbacks(paragraph, &font, size, &options, &fallbacks);
        widest = widest.max(layout.width);
        let down = row as f32
            * if line_height > 0.0 {
                size * line_height
            } else {
                natural
            };
        for line in &layout.lines {
            for g in &line.glyphs {
                // A glyph the face lacks draws nothing rather than its missing-glyph box.
                if g.glyph_id == 0 {
                    continue;
                }
                outline.x = g.x;
                outline.y = down + (g.y - line.baseline_y);
                // From the face it is drawn in; a colour emoji has no outline and draws nothing here.
                let face = match g.face {
                    blinc_text::FaceChoice::Primary => Some(&font),
                    choice => fallbacks.face(choice),
                };
                let Some(face) = face else {
                    continue;
                };
                let Some(swash_font) =
                    swash::FontRef::from_index(face.data(), face.face_index() as usize)
                else {
                    continue;
                };
                let mut builder = context.builder(swash_font).size(size).hint(false);
                if let Some(w) = face.variation_weight() {
                    builder = builder.variations([swash::Setting::from(("wght", w))]);
                }
                if let Some(path) = builder.build().scale_outline(g.glyph_id) {
                    outline.add(&path);
                }
            }
        }
    }
    if !info.is_null() {
        let info = info as *mut f32;
        for (i, v) in [widest, ascent, descent, natural].into_iter().enumerate() {
            unsafe { info.add(i).write_unaligned(v) };
        }
    }
    let n = outline.out.len();
    if !out.is_null() && n <= capacity.max(0) as usize {
        unsafe { std::ptr::copy_nonoverlapping(outline.out.as_ptr(), out as *mut f32, n) };
    }
    n as i32
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_text_outline, hl_blinc_text_outline, "PBBBBiB_i");

/// What shapes a run: the face, its style, the size and the letter spacing.
#[derive(Hash, PartialEq, Eq, Clone)]
struct StyleKey {
    name: Option<String>,
    generic: GenericFont,
    weight: u16,
    italic: bool,
    size: u32,
    spacing: u32,
    hint: bool,
}

/// A piece of a line shaped once: its advance and, per subpixel phase it has been placed at, its glyphs as nine f32s
/// each, relative to the run origin. A unit is one character, or the characters that must be shaped together.
#[derive(Default)]
struct Unit {
    advance: f32,
    phases: [Option<Vec<f32>>; 3],
}

/// The first printable ASCII character, and how many there are: those units and pairs live in arrays.
const ASCII_FIRST: u32 = 32;
const ASCII_COUNT: usize = 95;

/// What one style has learned: its units, and for pairs of characters what shaping them side by side adds.
/// A pair's value is NaN until it is known, infinity when the two must be shaped together, else the kerning.
struct StyleCache {
    ascii: Vec<Unit>,
    others: HashMap<String, Unit>,
    ascii_pairs: Vec<f32>,
    other_pairs: HashMap<(char, char), f32>,
}

impl StyleCache {
    fn new() -> Self {
        let mut cache = Self {
            ascii: Vec::new(),
            others: HashMap::new(),
            ascii_pairs: Vec::new(),
            other_pairs: HashMap::new(),
        };
        cache.reset();
        cache
    }

    fn reset(&mut self) {
        self.ascii.clear();
        self.ascii.resize_with(ASCII_COUNT, Unit::default);
        self.others.clear();
        self.ascii_pairs.clear();
        self.ascii_pairs.resize(ASCII_COUNT * ASCII_COUNT, f32::NAN);
        self.other_pairs.clear();
    }
}

/// Styles already learned. Atlas rectangles are inside them, so it is emptied whenever the atlases are cleared.
#[derive(Default)]
struct WordCache {
    styles: HashMap<StyleKey, StyleCache>,
    units: usize,
    generation: i32,
}

/// The most units kept before the cache starts again.
const MAX_CACHED_UNITS: usize = 100_000;

impl WordCache {
    fn check_generation(&mut self) {
        let now = ATLAS_GENERATION.load(Ordering::Relaxed);
        if now != self.generation || self.units >= MAX_CACHED_UNITS {
            self.styles.clear();
            self.units = 0;
            self.generation = now;
        }
    }
}

static WORD_CACHE: LazyLock<Mutex<WordCache>> = LazyLock::new(|| Mutex::new(WordCache::default()));

/// Whether lines are composed from cached units. Off only for tests that compare with shaping whole lines.
static WORD_CACHE_ENABLED: AtomicBool = AtomicBool::new(true);

struct RunStyle<'a> {
    key: StyleKey,
    name: Option<&'a str>,
    generic: GenericFont,
    weight: u16,
    italic: bool,
    size: f32,
    options: &'a LayoutOptions,
    /// Round each unit's advance to a whole pixel, as a hinted layout does; kerning stays fractional.
    hint: bool,
}

/// Characters whose glyphs depend on their neighbours beyond kerning: scripts that join, reorder or stack
/// (Hebrew through Myanmar, Khmer and the rest to U+1DFF), Hangul jamo, emoji. A run of them is shaped as one unit.
fn needs_context(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x109F | 0x1100..=0x11FF | 0x1700..=0x1DFF | 0x2190..=0x21FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF
        | 0x1F000..=0x1FAFF)
}

/// Characters of right-to-left scripts and the marks that set direction. A line holding any is reordered as a whole when
/// shaped, so it is one unit.
fn right_to_left(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF | 0x200F | 0x202B | 0x202E | 0x2067 | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF
        | 0x1E800..=0x1EFFF)
}

/// Characters that attach to the one before them: combining marks, joiners, variation selectors, emoji modifiers.
fn extends(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C..=0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF)
}

/// Characters a font kerns: Latin, Greek and Cyrillic and general punctuation. The rest are placed on their advances.
fn kerns(c: char) -> bool {
    matches!(c as u32, 0x0020..=0x058F | 0x2000..=0x206F)
}

fn ascii_index(c: char) -> Option<usize> {
    let c = c as u32;
    (c >= ASCII_FIRST && c < ASCII_FIRST + ASCII_COUNT as u32).then(|| (c - ASCII_FIRST) as usize)
}

fn glyph_records(prepared: &PreparedText) -> Vec<f32> {
    let mut out = Vec::with_capacity(prepared.glyphs.len() * 9);
    for g in &prepared.glyphs {
        let [gx, gy, gw, gh] = g.bounds;
        let [u0, v0, u1, v1] = g.uv_bounds;
        out.extend([gx, gy, gw, gh, u0, v0, u1, v1, if g.is_color { 1.0 } else { 0.0 }]);
    }
    out
}

/// Shapes `text` into the atlases with the origin at `origin` pixels, clearing the atlases once if they are full.
/// Returns whether it cleared them: everything the cache holds is stale then.
fn shape(
    r: &mut TextRenderer,
    run: &RunStyle,
    text: &str,
    origin: f32,
    cleared: &mut bool,
) -> Option<PreparedText> {
    let subpixel = Some(SubpixelX::new(SUBPIXEL_PHASES, origin));
    let attempt = |r: &mut TextRenderer| {
        r.prepare_text_subpixel(
            text,
            run.size,
            [1.0; 4],
            run.options,
            run.name,
            run.generic,
            run.weight,
            run.italic,
            subpixel,
        )
    };
    match attempt(r) {
        Ok(p) => Some(p),
        Err(TextError::AtlasFull) => {
            r.clear();
            note_atlas_cleared();
            *cleared = true;
            attempt(r).ok()
        }
        Err(_) => None,
    }
}

/// One line being composed: the renderer, the style's cache and where the unit count is kept.
struct Composer<'a> {
    r: &'a mut TextRenderer,
    run: &'a RunStyle<'a>,
    style: &'a mut StyleCache,
    units: &'a mut usize,
}

impl Composer<'_> {
    fn shape(&mut self, text: &str, origin: f32) -> Option<PreparedText> {
        let mut cleared = false;
        let prepared = shape(self.r, self.run, text, origin, &mut cleared);
        if cleared {
            self.style.reset();
            *self.units = 0;
        }
        prepared
    }

    /// What shaping `a` then `b` adds to their separate widths (the kerning), or infinity when they come out as other
    /// than one glyph each and so must be shaped together (a ligature).
    fn pair(&mut self, a: char, b: char) -> f32 {
        if !kerns(a) || !kerns(b) {
            return 0.0;
        }
        let ascii = ascii_index(a).zip(ascii_index(b));
        let known = match ascii {
            Some((i, j)) => self.style.ascii_pairs[i * ASCII_COUNT + j],
            None => self.style.other_pairs.get(&(a, b)).copied().unwrap_or(f32::NAN),
        };
        if !known.is_nan() {
            return known;
        }
        let both: String = [a, b].iter().collect();
        let shaped = self.shape(&both, 0.0);
        let wa = self.shape(&a.to_string(), 0.0).map_or(0.0, |p| p.width);
        let wb = self.shape(&b.to_string(), 0.0).map_or(0.0, |p| p.width);
        let kern = match shaped {
            Some(p) => {
                let expected = [a, b].iter().filter(|c| !c.is_whitespace()).count();
                if p.glyphs.len() == expected {
                    p.width - wa - wb
                } else {
                    f32::INFINITY
                }
            }
            None => 0.0,
        };
        match ascii {
            Some((i, j)) => self.style.ascii_pairs[i * ASCII_COUNT + j] = kern,
            None => {
                self.style.other_pairs.insert((a, b), kern);
            }
        }
        kern
    }

    /// The unit for `text` with its glyphs at `phase`, shaping it if it has not been.
    fn unit(&mut self, text: &str, single: Option<usize>, phase: usize) -> Option<&Unit> {
        let known = match single {
            Some(i) => self.style.ascii[i].phases[phase].is_some(),
            None => self.style.others.get(text).is_some_and(|u| u.phases[phase].is_some()),
        };
        if !known {
            let phases = SUBPIXEL_PHASES.get() as f32;
            let prepared = self.shape(text, phase as f32 / phases)?;
            let records = glyph_records(&prepared);
            let unit = match single {
                Some(i) => &mut self.style.ascii[i],
                None => self.style.others.entry(text.to_string()).or_default(),
            };
            if unit.phases.iter().all(Option::is_none) {
                *self.units += 1;
            }
            unit.advance = prepared.width;
            unit.phases[phase] = Some(records);
        }
        match single {
            Some(i) => self.style.ascii[i].phases[phase].is_some().then(|| &self.style.ascii[i]),
            None => self.style.others.get(text),
        }
    }
}

/// Appends the glyphs of one line to `out`, composed from the units the cache holds and shaping the ones it does
/// not. A unit is a character, or characters shaped together: those a font ligates, and runs of the scripts that
/// need their neighbours. Each is placed at its own pen position's whole pixel and nearest third of a pixel, as
/// shaping the line whole places a glyph, and kerning comes from the pair table. Returns the line's width.
fn compose_paragraph(
    composer: &mut Composer,
    paragraph: &str,
    down: f32,
    out: &mut Vec<f32>,
    mut advances: Option<&mut Vec<f32>>,
) -> f32 {
    let chars: Vec<char> = paragraph.chars().collect();
    let bidirectional = chars.iter().any(|&c| right_to_left(c));
    let phases = SUBPIXEL_PHASES.get() as f32;
    let mut pen = 0.0f32;
    let mut i = 0;
    let mut text = String::new();
    while i < chars.len() {
        let first = chars[i];
        let mut j = if bidirectional { chars.len() } else { i + 1 };
        while j < chars.len() {
            let next = chars[j];
            let joins = extends(next)
                || (needs_context(first) && needs_context(next))
                || (!needs_context(first) && !needs_context(next) && composer.pair(chars[j - 1], next).is_infinite());
            if !joins {
                break;
            }
            j += 1;
        }
        // The kerning with what came before, which is a plain pair: a joining one would have been taken into the last unit.
        if i > 0 && !needs_context(first) && !needs_context(chars[i - 1]) {
            let kern = composer.pair(chars[i - 1], first);
            if kern.is_finite() {
                pen += kern;
                if let Some(last) = advances.as_deref_mut().and_then(|a| a.last_mut()) {
                    *last += kern;
                }
            }
        }
        let single = if j == i + 1 { ascii_index(first) } else { None };
        text.clear();
        text.extend(&chars[i..j]);

        let mut whole = pen.floor();
        let mut phase = ((pen - whole) * phases).round() as usize;
        if phase >= phases as usize {
            whole += 1.0;
            phase = 0;
        }
        let hint = composer.run.hint;
        if let Some(unit) = composer.unit(&text, single, phase) {
            if let Some(records) = &unit.phases[phase] {
                for g in records.chunks_exact(9) {
                    out.extend([g[0] + whole, g[1] + down, g[2], g[3], g[4], g[5], g[6], g[7], g[8]]);
                }
            }
            let advance = if hint { unit.advance.round() } else { unit.advance };
            pen += advance;
            if let Some(a) = advances.as_deref_mut() {
                let share = advance / (j - i) as f32;
                a.extend(std::iter::repeat(share).take(j - i));
            }
        } else if let Some(a) = advances.as_deref_mut() {
            a.extend(std::iter::repeat(0.0).take(j - i));
        }
        i = j;
    }
    pen
}

/// Appends the glyphs of one line shaped as a whole, the reference `compose_paragraph` is checked against.
fn whole_paragraph(r: &mut TextRenderer, run: &RunStyle, paragraph: &str, down: f32, out: &mut Vec<f32>) -> f32 {
    let mut cleared = false;
    let Some(prepared) = shape(r, run, paragraph, 0.0, &mut cleared) else {
        return 0.0;
    };
    for g in glyph_records(&prepared).chunks_exact(9) {
        out.extend([g[0], g[1] + down, g[2], g[3], g[4], g[5], g[6], g[7], g[8]]);
    }
    prepared.width
}

/// The advance of each character of `text` in the face and style `prepare_glyphs` takes, as f32s in `out` (at most
/// `capacity`), and the line's width as one f32 in `info`. Returns how many characters there are. A `char` here is a
/// Unicode scalar value; a caller indexing UTF-16 gives the second half of a surrogate pair no advance.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_text_advances(
    text: *const vbyte,
    font_name: *const vbyte,
    style: *const vbyte,
    out: *mut vbyte,
    capacity: i32,
    info: *mut vbyte,
    flags: i32,
) -> i32 {
    if style.is_null() {
        return 0;
    }
    let s = |i: usize| unsafe { (style as *const f32).add(i).read_unaligned() };
    let style = [s(0), s(1), s(2), s(3), s(4), s(5)];
    let text = unsafe { crate::hl::string_from(text) };
    let name = unsafe { crate::hl::opt_string_from(font_name) };
    let mut advances = Vec::new();
    let mut metrics = [0.0f32; 4];
    unsafe {
        prepare_glyphs_with(&text, name, style, flags & HINT != 0, std::ptr::null_mut(), 0, metrics.as_mut_ptr() as *mut u8, Some(&mut advances));
    }
    if !info.is_null() {
        unsafe { (info as *mut f32).write_unaligned(metrics[0]) };
    }
    let n = advances.len();
    if !out.is_null() && n <= capacity.max(0) as usize {
        unsafe { std::ptr::copy_nonoverlapping(advances.as_ptr(), out as *mut f32, n) };
    }
    n as i32
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_text_advances, hl_blinc_text_advances, "PBBBBiBi_i");

/// Turns composing lines from cached units on or off; `1` is the default. Off shapes every line whole, so tests can
/// compare the two. Returns the previous setting.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub extern "C" fn hl_blinc_text_word_cache(enabled: i32) -> i32 {
    WORD_CACHE_ENABLED.swap(enabled != 0, Ordering::Relaxed) as i32
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_text_word_cache, hl_blinc_text_word_cache, "Pi_i");

/// `text` laid out and rasterized into the glyph atlases in a face of `font_name`
/// (else the generic family), as `style` sets it, six f32s: generic family (0
/// system, 1 monospace, 2 serif, 3 sans-serif), weight, italic (0 or 1), size
/// in pixels, letter spacing and line height. Writes nine f32s per glyph to `out`:
/// `[x, y, width, height, u0, v0, u1, v1, is_color]`, where `(x, y, width, height)`
/// are in pixels from the top-left of the text box and `(u0, v0, u1, v1)` are the
/// glyph's rectangle in its atlas in pixels (`is_color` is 1.0 for the colour-glyph
/// atlas, 0.0 for the coverage atlas). Writes the widest line's width, the face's
/// ascent and descent (positive, below the baseline) and its line height, in pixels,
/// as four f32s to `info`. Returns the number of f32s (`glyph_count * 9`), copying
/// them to `out` when they fit in `capacity`; 0 when no face is found.
pub unsafe fn prepare_glyphs(
    text: &str,
    name: Option<String>,
    style: [f32; 6],
    out: *mut u8,
    capacity: i32,
    info: *mut u8,
) -> i32 {
    unsafe { prepare_glyphs_with(text, name, style, false, out, capacity, info, None) }
}

/// `prepare_glyphs` with `hint`, which rounds each character's advance to a whole pixel, kerning excepted, as a hinted
/// layout does (lines shaped whole, with the cache off, are not hinted); and when `advances` is given, one advance per character of `text` in it: the distance from the
/// character's origin to the next one's, kerning included, the shares of a unit shaped together (a ligature) equal,
/// and a line break 0. Lines are only composed from cached units for this: with the cache off it is left empty.
pub unsafe fn prepare_glyphs_with(
    text: &str,
    name: Option<String>,
    style: [f32; 6],
    hint: bool,
    out: *mut u8,
    capacity: i32,
    info: *mut u8,
    mut advances: Option<&mut Vec<f32>>,
) -> i32 {
    let s = |i: usize| style[i];
    let (generic, weight, italic, size, letter_spacing, line_height) =
        (s(0) as i32, s(1) as i32, s(2) != 0.0, s(3), s(4), s(5));
    if size <= 0.0 {
        return 0;
    }
    let generic = match generic {
        1 => GenericFont::Monospace,
        2 => GenericFont::Serif,
        3 => GenericFont::SansSerif,
        _ => GenericFont::System,
    };
    let name = name.or_else(|| {
        if generic == GenericFont::System {
            system_ui()
        } else {
            None
        }
    });
    let weight = weight.clamp(1, 1000) as u16;
    let (ascent, descent, natural) = {
        let registry = global_font_registry();
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        let font = match registry.get_for_render_with_style(name.as_deref(), generic, weight, italic) {
            Some(font) => font,
            None => match registry.load_with_fallback_styled(
                name.as_deref(),
                generic,
                weight,
                italic,
            ) {
                Ok(font) => font,
                Err(_) => return 0,
            },
        };
        let m = font.metrics();
        (
            m.ascender_px(size),
            -m.descender_px(size),
            m.line_height_px(size),
        )
    };
    let options = LayoutOptions {
        max_width: None,
        alignment: TextAlignment::Left,
        anchor: TextAnchor::Top,
        line_break: LineBreakMode::None,
        line_height: 1.0,
        letter_spacing,
    };
    let step = if line_height > 0.0 {
        size * line_height
    } else {
        natural
    };
    let mut r = renderer();
    let mut glyphs_out: Vec<f32> = Vec::new();
    let mut widest = 0.0f32;
    let mut cache = WORD_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.check_generation();
    let run = RunStyle {
        key: StyleKey {
            name: name.clone(),
            generic,
            weight,
            italic,
            size: size.to_bits(),
            spacing: letter_spacing.to_bits(),
            hint,
        },
        name: name.as_deref(),
        generic,
        weight,
        italic,
        size,
        options: &options,
        hint,
    };
    let cached = WORD_CACHE_ENABLED.load(Ordering::Relaxed);
    {
        let WordCache { styles, units, .. } = &mut *cache;
        let style = styles.entry(run.key.clone()).or_insert_with(StyleCache::new);
        let mut composer = Composer {
            r: &mut r,
            run: &run,
            style,
            units,
        };
        for (row, paragraph) in text.split('\n').enumerate() {
            if paragraph.is_empty() {
                if row > 0 {
                    if let Some(a) = advances.as_deref_mut() {
                        a.push(0.0);
                    }
                }
                continue;
            }
            let down = row as f32 * step;
            if row > 0 {
                if let Some(a) = advances.as_deref_mut() {
                    a.push(0.0);
                }
            }
            let width = if cached {
                compose_paragraph(&mut composer, paragraph, down, &mut glyphs_out, advances.as_deref_mut())
            } else {
                whole_paragraph(composer.r, &run, paragraph, down, &mut glyphs_out)
            };
            widest = widest.max(width);
        }
    }
    drop(cache);
    if !info.is_null() {
        let info = info as *mut f32;
        for (i, v) in [widest, ascent, descent, natural].into_iter().enumerate() {
            unsafe { info.add(i).write_unaligned(v) };
        }
    }
    let n = glyphs_out.len();
    if !out.is_null() && n <= capacity.max(0) as usize {
        unsafe { std::ptr::copy_nonoverlapping(glyphs_out.as_ptr(), out as *mut f32, n) };
    }
    n as i32
}
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn hl_blinc_text_prepare(
    text: *const vbyte,
    font_name: *const vbyte,
    style: *const vbyte,
    out: *mut vbyte,
    capacity: i32,
    info: *mut vbyte,
    flags: i32,
) -> i32 {
    if style.is_null() {
        return 0;
    }
    let s = |i: usize| unsafe { (style as *const f32).add(i).read_unaligned() };
    let style = [s(0), s(1), s(2), s(3), s(4), s(5)];
    let text = unsafe { crate::hl::string_from(text) };
    let name = unsafe { crate::hl::opt_string_from(font_name) };
    unsafe { prepare_glyphs_with(&text, name, style, flags & HINT != 0, out, capacity, info, None) }
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_text_prepare, hl_blinc_text_prepare, "PBBBBiBi_i");

/// The number of times the glyph atlases have been cleared; see `ATLAS_GENERATION`.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub extern "C" fn hl_blinc_text_generation() -> i32 {
    ATLAS_GENERATION.load(Ordering::Relaxed)
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_text_generation, hl_blinc_text_generation, "P_i");
