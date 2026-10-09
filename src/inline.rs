//! Inline formatting: a paragraph of text runs in different styles, and
//! inline boxes, laid out as one flow. They wrap together at the
//! paragraph's width, each line's pieces sit on one baseline whatever their
//! font, and lines are aligned as CSS's `text-align` puts them.
//!
//! Whitespace collapses as HTML's does: a run of it is one space, and none
//! starts or ends a line. Lines break greedily at spaces; a word longer than
//! the line breaks inside only when `break_words` is set, as CSS's
//! `overflow-wrap: anywhere` allows. Indices are UTF-16, into each run's
//! collapsed text.

use crate::text::measure;
use blinc_layout::tree::TextMeasureContext;

/// What a flow lays out, in order.
pub enum InlineItem<'a> {
    /// Text in one style. `padding` is the left and right padding of a box
    /// painted around it (CSS's `code` background, say), at the run's start
    /// and end only, not where a line breaks it; a padded run's words keep
    /// their spaces when the line is justified.
    Text {
        context: &'a TextMeasureContext,
        text: &'a str,
        letter_spacing: f32,
        padding: [f32; 2],
    },
    /// An element kept whole: an image, an input, or an inline box. `baseline`
    /// is from its top to the baseline it sits on. A block box takes a line
    /// of its own.
    Box {
        width: f32,
        height: f32,
        baseline: f32,
        block: bool,
    },
    /// A line break, as HTML's `br`.
    Break,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineAlign {
    Left,
    Center,
    Right,
    /// Lines but the last, and those before a break, fill the width by
    /// widening the spaces between words.
    Justify,
}

/// One placed piece: a text run's characters `start..end` of its collapsed
/// text, or a whole box (`start` and `end` 0), with its top-left and size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineFragment {
    pub item: u32,
    pub start: u32,
    pub end: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineLine {
    pub top: f32,
    pub height: f32,
    /// From the line's top to its baseline.
    pub baseline: f32,
}

#[derive(Clone, Debug, Default)]
pub struct InlineLayout {
    pub fragments: Vec<InlineFragment>,
    pub lines: Vec<InlineLine>,
    /// Each text item's collapsed text, which fragments index; empty for others.
    pub texts: Vec<String>,
    /// The flow's natural width: its longest line, unwrapped.
    pub max_content: f32,
    /// The width it cannot be narrower than: its widest word or box, or 0
    /// when words may break.
    pub min_content: f32,
    pub height: f32,
}

/// A measured text run: its collapsed text as UTF-16, the x at each index,
/// and how far it reaches above and below its baseline.
struct Run {
    units: Vec<u16>,
    stops: Vec<f32>,
    above: f32,
    below: f32,
    padding: [f32; 2],
}
impl Run {
    fn x(&self, i: usize) -> f32 {
        self.stops[i.min(self.stops.len() - 1)]
    }
}

#[derive(Clone, Copy)]
enum Piece {
    /// A word of run `item`, `start..end`, its width (padding included at a
    /// padded run's ends) and the width of the space after it.
    Word {
        item: usize,
        start: usize,
        end: usize,
        width: f32,
        space: f32,
    },
    Box(usize),
    Break,
}

struct Line {
    pieces: Vec<(Piece, f32)>,
    above: f32,
    below: f32,
    /// The last line, or one before a break: justifying leaves it as it is.
    ends: bool,
}

fn is_space(c: u16) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0a | 0x0d | 0x0c)
}

/// Lay `items` out in a box `width` wide.
pub fn layout(
    items: &[InlineItem<'_>],
    width: f32,
    align: InlineAlign,
    break_words: bool,
) -> InlineLayout {
    let mut out = InlineLayout {
        texts: vec![String::new(); items.len()],
        ..Default::default()
    };
    // Collapse whitespace across the flow and measure each run.
    let mut runs: Vec<Option<Run>> = Vec::with_capacity(items.len());
    let mut after_space = true;
    for (i, item) in items.iter().enumerate() {
        match item {
            InlineItem::Text {
                context,
                text,
                letter_spacing,
                padding,
            } => {
                let mut units: Vec<u16> = Vec::new();
                let mut in_space = false;
                for c in text.encode_utf16() {
                    if is_space(c) {
                        in_space = true;
                        continue;
                    }
                    if in_space && !(units.is_empty() && after_space) {
                        units.push(0x20);
                    }
                    in_space = false;
                    units.push(c);
                }
                if in_space && !(units.is_empty() && after_space) {
                    units.push(0x20);
                }
                if let Some(&last) = units.last() {
                    after_space = last == 0x20;
                }
                let collapsed = String::from_utf16_lossy(&units);
                let mut context = (*context).clone();
                context.wrap = false;
                let measured = measure(&context, &collapsed, *letter_spacing, None);
                let mut stops = vec![0.0f32; units.len() + 1];
                let (mut above, mut below) = (0.0, 0.0);
                if let Some(m) = measured {
                    // Indices inside a character, the second half of a
                    // surrogate pair, stand where it starts.
                    let (mut at, mut last) = (0usize, 0.0f32);
                    for c in &m.carets {
                        let index = c.index as usize;
                        while at < index && at <= units.len() {
                            stops[at] = last;
                            at += 1;
                        }
                        if index <= units.len() {
                            stops[index] = c.x;
                        }
                        last = c.x;
                        at = index + 1;
                    }
                    while at <= units.len() {
                        stops[at] = last;
                        at += 1;
                    }
                    // As text is drawn: half the leading above the ascender.
                    above = (m.line_height - (m.ascender - m.descender)) / 2.0 + m.ascender;
                    below = m.line_height - above;
                }
                out.texts[i] = collapsed;
                runs.push(Some(Run {
                    units,
                    stops,
                    above,
                    below,
                    padding: *padding,
                }));
            }
            InlineItem::Box { .. } => {
                after_space = false;
                runs.push(None);
            }
            InlineItem::Break => {
                after_space = true;
                runs.push(None);
            }
        }
    }

    // The words of each run, between the boxes and breaks.
    let mut pieces = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item {
            InlineItem::Text { .. } => {
                let run = runs[i].as_ref().expect("measured above");
                let n = run.units.len();
                let mut start = 0;
                while start < n {
                    let end = run.units[start..]
                        .iter()
                        .position(|&c| c == 0x20)
                        .map_or(n, |p| start + p);
                    let space = if end < n {
                        run.x(end + 1) - run.x(end)
                    } else {
                        0.0
                    };
                    let mut w = run.x(end) - run.x(start);
                    if start == 0 {
                        w += run.padding[0];
                    }
                    if end == n {
                        w += run.padding[1];
                    }
                    pieces.push(Piece::Word {
                        item: i,
                        start,
                        end,
                        width: w,
                        space,
                    });
                    start = end + 1;
                }
            }
            InlineItem::Box { .. } => pieces.push(Piece::Box(i)),
            InlineItem::Break => pieces.push(Piece::Break),
        }
    }
    let padded = |item: usize| runs[item].as_ref().is_some_and(|r| r.padding != [0.0, 0.0]);
    let box_of = |item: usize| match items[item] {
        InlineItem::Box {
            width,
            height,
            baseline,
            block,
        } => (width, height, baseline, block),
        _ => (0.0, 0.0, 0.0, false),
    };
    let piece_width = |p: &Piece| match *p {
        Piece::Word { width, .. } => width,
        Piece::Box(i) => box_of(i).0,
        Piece::Break => 0.0,
    };

    // As wide as the longest line can be, no narrower than the longest word.
    let (mut longest, mut widest, mut x, mut trailing) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in &pieces {
        match *p {
            Piece::Word { width, space, .. } => {
                widest = widest.max(width);
                x += width + space;
                trailing = space;
            }
            Piece::Box(i) if box_of(i).3 => {
                longest = longest.max(x - trailing);
                x = 0.0;
                trailing = 0.0;
            }
            Piece::Box(i) => {
                widest = widest.max(box_of(i).0);
                x += box_of(i).0;
                trailing = 0.0;
            }
            Piece::Break => {
                longest = longest.max(x - trailing);
                x = 0.0;
                trailing = 0.0;
            }
        }
    }
    out.max_content = longest.max(x - trailing).ceil();
    out.min_content = if break_words { 0.0 } else { widest.ceil() };

    // Lines: greedy, breaking at spaces, a space at the start of a line dropped.
    let mut lines: Vec<Line> = Vec::new();
    let mut line = Line {
        pieces: Vec::new(),
        above: 0.0,
        below: 0.0,
        ends: false,
    };
    let mut x = 0.0f32;
    let mut queue: Vec<Piece> = pieces.into_iter().rev().collect();
    while let Some(mut piece) = queue.pop() {
        // A word too long for any line, where words may break: as much as
        // fits in the room left, the rest after.
        if let Piece::Word {
            item,
            start,
            end,
            width: w,
            space,
        } = piece
            && break_words
            && w > width
            && !padded(item)
            && end - start > 1
        {
            let run = runs[item].as_ref().expect("text run");
            let room = width - x;
            let mut to = start;
            while to < end && run.x(to + 1) - run.x(start) <= room {
                to += 1;
            }
            if to == start && x == 0.0 {
                to = start + 1;
            }
            if to > start && to < end {
                queue.push(Piece::Word {
                    item,
                    start: to,
                    end,
                    width: run.x(end) - run.x(to),
                    space,
                });
                piece = Piece::Word {
                    item,
                    start,
                    end: to,
                    width: run.x(to) - run.x(start),
                    space: 0.0,
                };
            }
        }
        let w = piece_width(&piece);
        match piece {
            Piece::Break => {
                line.ends = true;
                lines.push(std::mem::replace(
                    &mut line,
                    Line {
                        pieces: Vec::new(),
                        above: 0.0,
                        below: 0.0,
                        ends: false,
                    },
                ));
                x = 0.0;
                continue;
            }
            // A space alone at the start of a line.
            Piece::Word { width: 0.0, .. } if x == 0.0 => continue,
            _ => {}
        }
        let block = matches!(piece, Piece::Box(i) if box_of(i).3);
        if (x > 0.0 && x + w > width + 0.01) || (block && !line.pieces.is_empty()) {
            lines.push(std::mem::replace(
                &mut line,
                Line {
                    pieces: Vec::new(),
                    above: 0.0,
                    below: 0.0,
                    ends: false,
                },
            ));
            x = 0.0;
        }
        line.pieces.push((piece, x));
        if block {
            line.ends = true;
            lines.push(std::mem::replace(
                &mut line,
                Line {
                    pieces: Vec::new(),
                    above: 0.0,
                    below: 0.0,
                    ends: false,
                },
            ));
            x = 0.0;
            continue;
        }
        x += w + match piece {
            Piece::Word { space, .. } => space,
            _ => 0.0,
        };
    }
    if !line.pieces.is_empty() || lines.is_empty() {
        line.ends = true;
        lines.push(line);
    }
    // An empty line is as tall as the first run around it.
    let strut = runs.iter().flatten().next().map(|r| (r.above, r.below));
    for l in &mut lines {
        for (p, _) in &l.pieces {
            let (a, b) = match *p {
                Piece::Word { item, .. } => {
                    let r = runs[item].as_ref().expect("text run");
                    (r.above, r.below)
                }
                Piece::Box(i) => {
                    let (_, h, base, _) = box_of(i);
                    (base, h - base)
                }
                Piece::Break => (0.0, 0.0),
            };
            l.above = l.above.max(a);
            l.below = l.below.max(b);
        }
        if l.above + l.below == 0.0
            && let Some((a, b)) = strut
        {
            l.above = a;
            l.below = b;
        }
    }

    // Place each line's pieces.
    let mut y = 0.0f32;
    for (n, l) in lines.iter().enumerate() {
        // A space stretches unless it is inside a padded run, which keeps
        // its words together.
        let stretches = |k: usize| match l.pieces[k].0 {
            Piece::Word { item, space, .. } if space > 0.0 && k + 1 < l.pieces.len() => {
                !padded(item)
                    || !matches!(l.pieces[k + 1].0, Piece::Word { item: next, .. } if next == item)
            }
            _ => false,
        };
        let mut natural = 0.0f32;
        let mut gaps = 0;
        for (k, (p, px)) in l.pieces.iter().enumerate() {
            natural = natural.max(px + piece_width(p));
            if stretches(k) {
                gaps += 1;
            }
        }
        let free = (width - natural).max(0.0);
        let justify = align == InlineAlign::Justify && !l.ends && gaps > 0;
        let shift = match align {
            InlineAlign::Center => free / 2.0,
            InlineAlign::Right => free,
            _ => 0.0,
        };
        let extra = if justify { free / gaps as f32 } else { 0.0 };
        let mut xs = Vec::with_capacity(l.pieces.len());
        let mut gained = 0.0;
        for (k, (_, px)) in l.pieces.iter().enumerate() {
            xs.push(px + shift + gained);
            if justify && stretches(k) {
                gained += extra;
            }
        }
        let mut i = 0;
        while i < l.pieces.len() {
            match l.pieces[i].0 {
                Piece::Word { item, start, .. } => {
                    // The run's words on this line, one fragment; each word
                    // its own when the spaces between them are widened.
                    let mut j = i;
                    let mut end = start;
                    while j < l.pieces.len() {
                        match l.pieces[j].0 {
                            Piece::Word {
                                item: r, end: e, ..
                            } if r == item && (j == i || !justify || padded(item)) => {
                                end = e;
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    let run = runs[item].as_ref().expect("text run");
                    let right = xs[j - 1] + piece_width(&l.pieces[j - 1].0);
                    out.fragments.push(InlineFragment {
                        item: item as u32,
                        start: start as u32,
                        end: end as u32,
                        x: xs[i],
                        y: y + l.above - run.above,
                        width: right - xs[i],
                        height: run.above + run.below,
                        line: n as u32,
                    });
                    i = j;
                }
                Piece::Box(b) => {
                    let (w, h, base, _) = box_of(b);
                    out.fragments.push(InlineFragment {
                        item: b as u32,
                        start: 0,
                        end: 0,
                        x: xs[i],
                        y: y + l.above - base,
                        width: w,
                        height: h,
                        line: n as u32,
                    });
                    i += 1;
                }
                Piece::Break => i += 1,
            }
        }
        out.lines.push(InlineLine {
            top: y,
            height: l.above + l.below,
            baseline: l.above,
        });
        y += l.above + l.below;
    }
    out.height = y.ceil();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use blinc_layout::div::GenericFont;

    fn context(size: f32, weight: u16) -> TextMeasureContext {
        let context = TextMeasureContext {
            content: String::new(),
            font_size: size,
            line_height: 1.2,
            letter_spacing: 0.0,
            wrap: false,
            font_name: None,
            generic_font: GenericFont::System,
            font_weight: weight,
            italic: false,
        };
        crate::text::ensure_face(&context);
        context
    }
    fn text<'a>(context: &'a TextMeasureContext, text: &'a str) -> InlineItem<'a> {
        InlineItem::Text {
            context,
            text,
            letter_spacing: 0.0,
            padding: [0.0, 0.0],
        }
    }

    #[test]
    fn runs_wrap_together_on_one_baseline() {
        let small = context(14.0, 400);
        let large = context(28.0, 700);
        let items = [
            text(&small, "  The  quick\nbrown "),
            text(&large, " fox jumps"),
            text(&small, " over the lazy dog"),
        ];
        let one = layout(&items, 10_000.0, InlineAlign::Left, false);
        // Whitespace collapses across runs: no leading space, one between words.
        assert_eq!(one.texts[0], "The quick brown ");
        assert_eq!(one.texts[1], "fox jumps");
        assert_eq!(one.lines.len(), 1);
        // Every fragment sits on the line's baseline: the big run's top is higher.
        let [a, b, c] = [0, 1, 2].map(|i| one.fragments.iter().find(|f| f.item == i).unwrap());
        assert!(b.y < a.y);
        assert!((a.y + a.height - c.y - c.height).abs() < 0.01);
        assert!(b.x > a.x && c.x > b.x);
        assert_eq!(one.max_content, (c.x + c.width).ceil());
        // Narrower than the line, it wraps between words of different runs.
        let wrapped = layout(&items, one.max_content * 0.45, InlineAlign::Left, false);
        assert!(wrapped.lines.len() >= 2);
        assert!(wrapped.height > one.height);
        for f in &wrapped.fragments {
            assert!(f.x + f.width <= one.max_content * 0.45 + 0.5 || f.x == 0.0);
        }
        // Right alignment moves lines to the right edge; centring halfway.
        let width = one.max_content + 100.0;
        let right = layout(&items, width, InlineAlign::Right, false);
        let center = layout(&items, width, InlineAlign::Center, false);
        assert!((right.fragments[0].x - 100.0).abs() < 0.5);
        assert!((center.fragments[0].x - 50.0).abs() < 0.5);
    }

    #[test]
    fn breaks_boxes_justify_and_long_words() {
        let ctx = context(16.0, 400);
        let items = [
            text(&ctx, "one two three four five six seven"),
            InlineItem::Break,
            InlineItem::Box {
                width: 30.0,
                height: 40.0,
                baseline: 40.0,
                block: false,
            },
            text(&ctx, " after"),
        ];
        let natural = layout(&items, 10_000.0, InlineAlign::Left, false);
        assert_eq!(natural.lines.len(), 2, "the break starts a line");
        let boxed = natural.fragments.iter().find(|f| f.item == 2).unwrap();
        assert_eq!((boxed.x, boxed.width, boxed.height), (0.0, 30.0, 40.0));
        assert!(
            natural.lines[1].baseline >= 40.0,
            "the box raises the line's baseline"
        );
        // Justified, a full line spans the width; the last line does not.
        let width = natural.fragments[0].width * 0.6;
        let justified = layout(&items, width, InlineAlign::Justify, false);
        let first = justified.fragments.iter().filter(|f| f.line == 0);
        let right = first.map(|f| f.x + f.width).fold(0.0f32, f32::max);
        assert!((right - width).abs() < 0.5, "{right} vs {width}");
        // A word longer than the line breaks only when words may break.
        let long = [text(&ctx, "Supercalifragilistic")];
        let kept = layout(&long, 40.0, InlineAlign::Left, false);
        assert_eq!(kept.lines.len(), 1);
        assert!(kept.min_content > 40.0);
        let broken = layout(&long, 40.0, InlineAlign::Left, true);
        assert!(broken.lines.len() > 1);
        assert_eq!(broken.min_content, 0.0);
    }
}
