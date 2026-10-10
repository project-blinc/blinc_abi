//! CSS colours: hex, names, `rgb()`, `hsl()`, `color-mix()` and `currentcolor`.

use super::quantity::amount;
use super::value::{call, dimension, split};
use blinc_core::Color;

/// CSS's named colours as 0xrrggbb, sorted by name.
const NAMED: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

fn rgb24(rgb: u32, alpha: f32) -> Color {
    Color::rgba(
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
        alpha,
    )
}

/// `text` as a colour; `current` is what `currentcolor` means.
pub fn parse(text: &str, current: Option<Color>) -> Result<Color, String> {
    let t = text.trim().to_ascii_lowercase();
    if t == "currentcolor" {
        return current.ok_or_else(|| "currentcolor needs a colour of the element's own".into());
    }
    if t == "transparent" {
        return Ok(Color::rgba(0.0, 0.0, 0.0, 0.0));
    }
    if let Some(digits) = t.strip_prefix('#') {
        return hex(digits, text);
    }
    if let Ok(i) = NAMED.binary_search_by(|(name, _)| (*name).cmp(t.as_str())) {
        return Ok(rgb24(NAMED[i].1, 1.0));
    }
    let Some((name, args)) = call(&t) else {
        return Err(format!("expected a colour, not \"{text}\""));
    };
    if name == "color-mix" {
        return mix(&args, current);
    }
    let parts = if args.contains(',') {
        split(&args, ',')
    } else {
        slashed(&args)?
    };
    let alpha = |parts: &[String]| -> Result<f32, String> {
        match parts.get(3) {
            Some(a) => Ok(amount(a)?.clamp(0.0, 1.0) as f32),
            None => Ok(1.0),
        }
    };
    match name.as_str() {
        "rgb" | "rgba" => {
            if parts.len() < 3 || parts.len() > 4 {
                return Err(format!(
                    "{name}() takes three channels and an optional alpha"
                ));
            }
            let c = [
                channel(&parts[0])?,
                channel(&parts[1])?,
                channel(&parts[2])?,
            ];
            Ok(rgb24((c[0] << 16) | (c[1] << 8) | c[2], alpha(&parts)?))
        }
        "hsl" | "hsla" => {
            if parts.len() < 3 || parts.len() > 4 {
                return Err(format!(
                    "{name}() takes a hue, saturation, lightness and an optional alpha"
                ));
            }
            let hue = match dimension(&parts[0]) {
                Some((v, u)) if u.is_empty() => v.to_radians(),
                _ => super::quantity::angle(&parts[0])?,
            };
            let s = amount(&parts[1])?.clamp(0.0, 1.0);
            let l = amount(&parts[2])?.clamp(0.0, 1.0);
            Ok(rgb24(hsl(hue, s, l), alpha(&parts)?))
        }
        _ => Err(format!(
            "{name}() is not a colour this supports; rgb(), hsl(), color-mix(), hex and names are"
        )),
    }
}

/// Space-separated channels with an optional `/ alpha`.
fn slashed(args: &str) -> Result<Vec<String>, String> {
    let halves: Vec<&str> = args.split('/').collect();
    if halves.len() > 2 {
        return Err("a colour has one / before its alpha".into());
    }
    let mut out = split(halves[0], ' ');
    if halves.len() == 2 {
        out.push(halves[1].trim().to_string());
    }
    Ok(out)
}

/// A channel, 0 to 255 or a percentage, rounded to a byte.
fn channel(text: &str) -> Result<u32, String> {
    match dimension(text) {
        Some((v, u)) if u.is_empty() || u == "%" => {
            let v = if u == "%" { v / 100.0 * 255.0 } else { v };
            Ok(v.round().clamp(0.0, 255.0) as u32)
        }
        _ => Err(format!(
            "expected a colour channel, 0 to 255 or a percentage, not \"{text}\""
        )),
    }
}

fn hex(digits: &str, text: &str) -> Result<Color, String> {
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("\"{text}\" is not a hex colour"));
    }
    let nibble = |i: usize| u32::from_str_radix(&digits[i..=i].repeat(2), 16).unwrap();
    let byte = |i: usize| u32::from_str_radix(&digits[i..i + 2], 16).unwrap();
    Ok(match digits.len() {
        3 => rgb24((nibble(0) << 16) | (nibble(1) << 8) | nibble(2), 1.0),
        4 => rgb24(
            (nibble(0) << 16) | (nibble(1) << 8) | nibble(2),
            nibble(3) as f32 / 255.0,
        ),
        6 => rgb24((byte(0) << 16) | (byte(2) << 8) | byte(4), 1.0),
        8 => rgb24(
            (byte(0) << 16) | (byte(2) << 8) | byte(4),
            byte(6) as f32 / 255.0,
        ),
        _ => {
            return Err(format!(
                "\"{text}\" is not a hex colour: 3, 4, 6 or 8 digits"
            ));
        }
    })
}

/// Hue in radians, saturation and lightness 0 to 1, as 0xrrggbb.
fn hsl(h: f64, s: f64, l: f64) -> u32 {
    let mut turns = (h / std::f64::consts::TAU) % 1.0;
    if turns < 0.0 {
        turns += 1.0;
    }
    let f = |n: f64| -> u32 {
        let k = (n + turns * 12.0) % 12.0;
        let a = s * l.min(1.0 - l);
        let v = l - a * (-1.0f64).max((k - 3.0).min(9.0 - k).min(1.0));
        (v * 255.0).round() as u32
    };
    (f(0.0) << 16) | (f(8.0) << 8) | f(4.0)
}

/// `color-mix(in srgb, a p%, b q%)`: premultiplied alpha, a missing
/// percentage the rest of 100, percentages under 100 scaling the alpha.
fn mix(args: &str, current: Option<Color>) -> Result<Color, String> {
    let parts = split(args, ',');
    if parts.len() != 3 || parts[0] != "in srgb" {
        return Err("color-mix() takes in srgb and two colours".into());
    }
    let mut sides = Vec::new();
    for part in &parts[1..] {
        let words = split(part, ' ');
        let last = words.last().map(String::as_str).unwrap_or("");
        let percent = (words.len() > 1 && last.ends_with('%'))
            .then(|| last[..last.len() - 1].parse::<f64>().map(|p| p / 100.0))
            .transpose()
            .map_err(|_| format!("expected a percentage, not \"{last}\""))?;
        let color = match percent {
            None => parse(part, current)?,
            Some(_) => parse(&words[..words.len() - 1].join(" "), current)?,
        };
        sides.push((color, percent));
    }
    let (mut p1, mut p2) = (sides[0].1, sides[1].1);
    match (p1, p2) {
        (None, None) => (p1, p2) = (Some(0.5), Some(0.5)),
        (None, Some(b)) => p1 = Some(1.0 - b),
        (Some(a), None) => p2 = Some(1.0 - a),
        _ => {}
    }
    let (p1, p2) = (p1.unwrap(), p2.unwrap());
    let sum = p1 + p2;
    if sum <= 0.0 {
        return Err("color-mix()'s percentages sum to zero".into());
    }
    let scale = sum.min(1.0);
    let (w1, w2) = (p1 / sum, p2 / sum);
    let (a, b) = (sides[0].0, sides[1].0);
    let alpha = a.a as f64 * w1 + b.a as f64 * w2;
    if alpha <= 0.0 {
        return Ok(Color::rgba(0.0, 0.0, 0.0, 0.0));
    }
    let mixed = |x: f32, y: f32| {
        ((x as f64 * a.a as f64 * w1 + y as f64 * b.a as f64 * w2) / alpha).clamp(0.0, 1.0) as f32
    };
    Ok(Color::rgba(
        mixed(a.r, b.r),
        mixed(a.g, b.g),
        mixed(a.b, b.b),
        (alpha * scale) as f32,
    ))
}
