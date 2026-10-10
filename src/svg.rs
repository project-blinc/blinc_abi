//! SVG rasterization, for ashui's SVG elements: ashui parses and checks the
//! markup itself and hands Blinc only what to rasterize, at the size it
//! covers on screen.

#[cfg(feature = "hashlink")]
use crate::hl::string_from;
use blinc_svg::{RasterizedSvg, SvgDocument};
#[cfg(feature = "hashlink")]
use hl_abi::{define_prim, vbyte};

/// Rasterizes `markup` into `width` × `height` straight-alpha RGBA pixels in
/// `out`, which holds at least `width * height * 4` bytes, its viewBox fitted
/// and centred. False when the markup does not parse.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_svg_rasterize(
    markup: *const vbyte,
    width: i32,
    height: i32,
    out: *mut vbyte,
) -> bool {
    if width <= 0 || height <= 0 || out.is_null() {
        return false;
    }
    let markup = unsafe { string_from(markup) };
    let Ok(raster) = RasterizedSvg::from_str(&markup, width as u32, height as u32) else {
        return false;
    };
    let size = (width * height * 4) as usize;
    if raster.pixels.len() != size {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(raster.pixels.as_ptr(), out as *mut u8, size) };
    true
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_svg_rasterize, hl_blinc_svg_rasterize, "PBiiB_b");

pub fn rasterize(markup: &str, width: u32, height: u32) -> Result<RasterizedSvg, String> {
    if width == 0 || height == 0 {
        return Err("Invalid SVG dimensions".into());
    }
    RasterizedSvg::from_str(markup, width, height).map_err(|e| e.to_string())
}

/// The size `markup` says its picture is, in whole pixels, at least one: its `width` and
/// `height`; with one of them, the other follows the viewBox's shape, as a browser sizes it; with
/// neither, the viewBox's size. An error when it is not SVG.
pub fn size(markup: &str) -> Result<(u32, u32), String> {
    let (mut width, mut height) = SvgDocument::from_str(markup)
        .map_err(|e| e.to_string())?
        .size();
    if let Some(root) = root_tag(markup) {
        let given = |name: &str| attribute(root, name).is_some();
        let view = attribute(root, "viewBox").and_then(|v| {
            let n: Vec<f32> = v
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|p| !p.is_empty())
                .filter_map(|p| p.parse().ok())
                .collect();
            (n.len() == 4 && n[2] > 0.0 && n[3] > 0.0).then(|| n[3] / n[2])
        });
        match (given("width"), given("height"), view) {
            (true, false, Some(ratio)) => height = width * ratio,
            (false, true, Some(ratio)) => width = height / ratio,
            _ => {}
        }
    }
    let whole = |v: f32| {
        if v.is_finite() {
            (v.ceil() as u32).max(1)
        } else {
            1
        }
    };
    Ok((whole(width), whole(height)))
}

/// The text of the first `<svg …>` start tag.
fn root_tag(markup: &str) -> Option<&str> {
    let lower = markup.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<svg") {
        let start = from + at;
        let after = markup[start + 4..].chars().next();
        if after.is_some_and(|c| c.is_whitespace() || c == '>' || c == '/') {
            let mut quote = None;
            for (i, c) in markup[start..].char_indices() {
                match (quote, c) {
                    (None, '"' | '\'') => quote = Some(c),
                    (Some(q), c) if c == q => quote = None,
                    (None, '>') => return Some(&markup[start..start + i + 1]),
                    _ => {}
                }
            }
            return None;
        }
        from = start + 4;
    }
    None
}

/// The value of attribute `name` in a start tag, if it has one.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().next_back();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) {
            let after = after.trim_start();
            if let Some(value) = after.strip_prefix('=') {
                let value = value.trim_start();
                if let Some(q) = value.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                    let value = &value[1..];
                    return value.find(q).map(|end| &value[..end]);
                }
            }
        }
        rest = &rest[at + name.len()..];
    }
    None
}
