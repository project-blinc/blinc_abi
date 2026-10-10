//! The CSS `filter` functions, as one value with a field per function.

use super::quantity::{PaintUnits, amount, angle, length};
use super::shadow;
use super::value::{call, split};
use blinc_core::Shadow;

/// One field per filter function, at its identity when the function is absent.
#[derive(Clone, Copy, Debug)]
pub struct Filter {
    pub brightness: f32,
    pub contrast: f32,
    pub grayscale: f32,
    /// Degrees.
    pub hue_rotate: f32,
    pub invert: f32,
    pub saturate: f32,
    pub sepia: f32,
    /// Standard deviation in pixels.
    pub blur: f32,
    pub drop_shadow: Option<Shadow>,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            brightness: 1.0,
            contrast: 1.0,
            grayscale: 0.0,
            hue_rotate: 0.0,
            invert: 0.0,
            saturate: 1.0,
            sepia: 0.0,
            blur: 0.0,
            drop_shadow: None,
        }
    }
}

impl Filter {
    /// Whether every function is at its identity.
    pub fn is_identity(&self) -> bool {
        let d = Filter::default();
        self.brightness == d.brightness
            && self.contrast == d.contrast
            && self.grayscale == d.grayscale
            && self.hue_rotate == d.hue_rotate
            && self.invert == d.invert
            && self.saturate == d.saturate
            && self.sepia == d.sepia
            && self.blur == d.blur
            && self.drop_shadow.is_none()
    }
}

/// `none`, or a list of filter functions; a function named twice keeps its last.
pub fn parse(text: &str, units: &PaintUnits) -> Result<Filter, String> {
    let mut filter = Filter::default();
    if text.trim().eq_ignore_ascii_case("none") {
        return Ok(filter);
    }
    for part in split(text, ' ') {
        let (name, args) =
            call(&part).ok_or_else(|| format!("expected a filter function, not \"{part}\""))?;
        let arg = args.trim();
        let unit = |v: f64| v as f32;
        match name.as_str() {
            "brightness" => filter.brightness = unit(amount(arg)?),
            "contrast" => filter.contrast = unit(amount(arg)?),
            "grayscale" => filter.grayscale = unit(amount(arg)?.clamp(0.0, 1.0)),
            "invert" => filter.invert = unit(amount(arg)?.clamp(0.0, 1.0)),
            "saturate" => filter.saturate = unit(amount(arg)?),
            "sepia" => filter.sepia = unit(amount(arg)?.clamp(0.0, 1.0)),
            "hue-rotate" => filter.hue_rotate = unit(angle(arg)?.to_degrees()),
            "blur" => {
                filter.blur = unit(if arg.is_empty() {
                    0.0
                } else {
                    length(arg, units)?
                })
            }
            "drop-shadow" => {
                let (inset, s) = shadow::parse_one(arg, false, units)?;
                debug_assert!(!inset);
                filter.drop_shadow = Some(s);
            }
            "opacity" => return Err("the opacity() filter is not supported; use opacity".into()),
            _ => return Err(format!("{name}() is not a filter")),
        }
    }
    Ok(filter)
}
