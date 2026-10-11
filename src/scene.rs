//! Owned scene encoding. Hosts supply storage; no GC values or borrowed pointers cross the ABI.
use crate::{
    context::{LayoutContext, Node},
    display_list::{self, Clip, Glyphs, RECORD_FLOATS, Shapes},
    tree::Scroll,
};
pub use blinc_core;
pub use blinc_layout::{element::RenderProps, tree::TextMeasureContext};
use blinc_text::TextRenderer;
use std::collections::VecDeque;

pub const DISPLAY_LIST_VERSION: u32 = 1;
pub type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy)]
pub struct PaintOptions {
    pub scale: f32,
    pub shapes: Shapes,
    pub text_color: [f32; 4],
}
impl Default for PaintOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            shapes: Shapes {
                n: 0.0,
                threshold: 0.0,
                radius_full: 9999.0,
            },
            text_color: [0.0, 0.0, 0.0, 1.0],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PaintInfo {
    pub count: usize,
    pub floats: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub node: Node,
    pub x: f32,
    pub y: f32,
}

impl LayoutContext {
    /// Bind a prepared renderer image to a brush source and fit (cover/contain/fill/tile).
    pub fn set_image_source(&mut self, source: String, fit: i32, slot: Option<i32>) -> Result<()> {
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if source.is_empty()
            || !(0..=3).contains(&fit)
            || slot.is_some_and(|v| !(0..=16_777_215).contains(&v))
        {
            return Err("Invalid image source, fit or slot");
        }
        if slot.is_some() {
            tree.image_sources.entry(source).or_default()[fit as usize] = slot;
        } else if let Some(slots) = tree.image_sources.get_mut(&source) {
            slots[fit as usize] = None;
            if slots.iter().all(Option::is_none) {
                tree.image_sources.remove(&source);
            }
        }
        self.revision += 1;
        Ok(())
    }
    pub fn properties(&self, node: Node) -> Result<RenderProps> {
        let id = self.check(node)?;
        Ok(self
            .tree
            .as_ref()
            .ok_or("Layout context is disposed")?
            .props
            .get(&id)
            .cloned()
            .unwrap_or_default())
    }
    pub fn set_properties(&mut self, node: Node, props: RenderProps) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        let source = |p: &RenderProps| match &p.background {
            Some(blinc_core::Brush::Image(i)) => Some(i.source.clone()),
            _ => None,
        };
        let now = source(&props);
        if tree.props.get(&id).and_then(source) != now {
            tree.image_uses.insert(id, now);
        }
        tree.props.insert(id, props);
        self.revision += 1;
        Ok(())
    }
    /// The nodes whose background image source has changed since this was last called, each once
    /// with the source it names now, or none when it names no image any more; a node removed
    /// since is not listed. In no particular order.
    pub fn take_image_uses(&mut self) -> Vec<(Node, Option<String>)> {
        let Some(tree) = self.tree.as_mut() else {
            return Vec::new();
        };
        let context = self.id;
        std::mem::take(&mut tree.image_uses)
            .into_iter()
            .map(|(id, source)| (Node { context, id }, source))
            .collect()
    }
    pub fn create_text(
        &mut self,
        style: crate::context::Style,
        text: TextMeasureContext,
    ) -> Result<Node> {
        validate_text(&text)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        crate::text::initialize();
        crate::text::ensure_face(&text);
        let id = tree.create_text_node(style, text);
        self.revision += 1;
        Ok(Node {
            context: self.id,
            id,
        })
    }
    pub fn text(&self, node: Node) -> Result<TextMeasureContext> {
        let id = self.check(node)?;
        self.tree
            .as_ref()
            .ok_or("Layout context is disposed")?
            .text_context(id)
            .cloned()
            .ok_or("Node is not text")
    }
    pub fn set_text(&mut self, node: Node, text: TextMeasureContext) -> Result<()> {
        let id = self.check(node)?;
        validate_text(&text)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if tree.text_context(id).is_none() {
            return Err("Node is not text");
        }
        crate::text::ensure_face(&text);
        tree.update_text(id, |current| *current = text);
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }
    pub fn set_visual(&mut self, node: Node, visual: Option<[f32; 4]>) -> Result<()> {
        let id = self.check(node)?;
        if visual.is_some_and(|v| !v.iter().all(|v| v.is_finite()) || (v[2] >= 0.0 && v[3] < 0.0)) {
            return Err("Invalid visual offset or size");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(v) = visual {
            tree.visuals.insert(id, v);
        } else {
            tree.visuals.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_scroll(&mut self, node: Node, scroll: Option<Scroll>) -> Result<()> {
        let id = self.check(node)?;
        if scroll.is_some_and(|s| {
            !s.x.is_finite()
                || !s.y.is_finite()
                || !s
                    .thumb
                    .iter()
                    .all(|c| c.is_finite() && (0.0..=1.0).contains(c))
        }) {
            return Err("Invalid scroll offset or color");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(s) = scroll {
            tree.scrolls.insert(id, s);
        } else {
            tree.scrolls.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_pointer_events(&mut self, node: Node, enabled: bool) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if enabled {
            tree.pass_through.remove(&id);
        } else {
            tree.pass_through.insert(id);
        }
        self.revision += 1;
        Ok(())
    }
    /// Caller-owned renderer slot, encoded in image/canvas records. None removes it.
    pub fn set_resource(&mut self, node: Node, slot: Option<i32>, canvas: bool) -> Result<()> {
        let id = self.check(node)?;
        if slot.is_some_and(|slot| !(0..=16_777_215).contains(&slot)) {
            return Err("Resource slot exceeds exact float range");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        let slots = if canvas {
            &mut tree.canvases
        } else {
            &mut tree.images
        };
        if let Some(slot) = slot {
            slots.insert(id, slot);
        } else {
            slots.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_glass_effects(
        &mut self,
        node: Node,
        effects: Option<crate::types::GlassEffects>,
    ) -> Result<()> {
        let id = self.check(node)?;
        if effects.is_some_and(|e| {
            !e.aberration.is_finite()
                || !(0.0..=1.0).contains(&e.aberration)
                || !e.bevel.is_finite()
                || e.bevel < 0.0
        }) {
            return Err("Invalid glass effects");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(effects) = effects {
            tree.glass_effects.insert(id, effects);
        } else {
            tree.glass_effects.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    /// Whether a node's clip-path polygon or path is filled by the even-odd rule, the nonzero
    /// rule otherwise.
    pub fn set_clip_even_odd(&mut self, node: Node, even_odd: bool) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if even_odd {
            tree.even_odd.insert(id);
        } else {
            tree.even_odd.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    /// How a node decorates its text and its descendants', drawn beside their ancestors'; none
    /// takes the node's own away.
    /// How many lines of text `node`, a block container, shows before the last ends with an
    /// ellipsis; 0 shows all, and none takes the setting away.
    pub fn set_line_clamp(&mut self, node: Node, lines: Option<u32>) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        match lines {
            Some(n) if n > 0 => tree.line_clamps.insert(id, n),
            _ => tree.line_clamps.remove(&id),
        };
        self.bounds.clear();
        self.revision += 1;
        Ok(())
    }

    /// What `node`, a block container, does with the text that overflows it: true ends it with an
    /// ellipsis, false clips it, and none takes the setting away.
    pub fn set_text_overflow(&mut self, node: Node, ellipsis: Option<bool>) -> Result<()> {
        let id = self.check(node)?;
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        match ellipsis {
            Some(on) => tree.text_overflows.insert(id, on),
            None => tree.text_overflows.remove(&id),
        };
        self.revision += 1;
        Ok(())
    }

    /// The shadows of `node`'s text and its descendants', first on top; an empty list for
    /// `none`, and none to take what an ancestor sets.
    pub fn set_text_shadow(
        &mut self,
        node: Node,
        shadows: Option<Vec<blinc_core::Shadow>>,
    ) -> Result<()> {
        let id = self.check(node)?;
        if shadows.as_ref().is_some_and(|list| {
            list.iter().any(|s| {
                !(s.offset_x.is_finite()
                    && s.offset_y.is_finite()
                    && s.blur.is_finite()
                    && s.blur >= 0.0)
            })
        }) {
            return Err("Invalid text shadow");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        match shadows {
            Some(list) => tree.text_shadows.insert(id, list),
            None => tree.text_shadows.remove(&id),
        };
        self.revision += 1;
        Ok(())
    }

    pub fn set_text_decoration(
        &mut self,
        node: Node,
        decoration: Option<crate::css::text_decoration::TextDecoration>,
    ) -> Result<()> {
        let id = self.check(node)?;
        if decoration.is_some_and(|d| {
            d.thickness.is_some_and(|t| !t.is_finite() || t < 0.0)
                || d.offset.is_some_and(|o| !o.is_finite())
        }) {
            return Err("Invalid text decoration");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(decoration) = decoration {
            tree.text_decorations.insert(id, decoration);
        } else {
            tree.text_decorations.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_backdrop_filters(&mut self, node: Node, filters: Option<[f32; 7]>) -> Result<()> {
        let id = self.check(node)?;
        if filters.is_some_and(|f| !f.iter().all(|v| v.is_finite())) {
            return Err("Invalid backdrop filters");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(filters) = filters {
            tree.backdrop_filters.insert(id, filters);
        } else {
            tree.backdrop_filters.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub fn set_notch(&mut self, node: Node, notch: Option<[[f32; 4]; 3]>) -> Result<()> {
        let id = self.check(node)?;
        if notch.is_some_and(|f| !f.iter().flatten().all(|v| v.is_finite())) {
            return Err("Invalid notch");
        }
        let tree = self.tree.as_mut().ok_or("Layout context is disposed")?;
        if let Some(notch) = notch {
            tree.notches.insert(id, notch);
        } else {
            tree.notches.remove(&id);
        }
        self.revision += 1;
        Ok(())
    }
    pub(crate) fn prepared_root(&self, root: Node) -> Result<blinc_layout::tree::LayoutNodeId> {
        let id = self.check(root)?;
        if !self.bounds.contains_key(&id) {
            return Err("Compute layout before painting or hit testing");
        }
        if self.parents.contains_key(&id) {
            return Err("Scene root must have no parent");
        }
        Ok(id)
    }
    pub fn hit_test(&self, root: Node, x: f32, y: f32) -> Result<Vec<Hit>> {
        let id = self.prepared_root(root)?;
        if !x.is_finite() || !y.is_finite() {
            return Err("Invalid hit coordinates");
        }
        Ok(crate::hit::test(
            self.tree.as_ref().ok_or("Layout context is disposed")?,
            id,
            x,
            y,
        )
        .into_iter()
        .map(|hit| Hit {
            node: Node {
                context: self.id,
                id: hit.node,
            },
            x: hit.x,
            y: hit.y,
        })
        .collect())
    }
    /// Exact hit path and a conservative rectangle reusable until any scene geometry changes.
    pub fn hit_test_region(
        &self,
        root: Node,
        x: f32,
        y: f32,
    ) -> Result<(Vec<Hit>, crate::hit::HitRegion)> {
        let id = self.prepared_root(root)?;
        if !x.is_finite() || !y.is_finite() {
            return Err("Invalid hit coordinates");
        }
        let (hits, region) = crate::hit::test_region(
            self.tree.as_ref().ok_or("Layout context is disposed")?,
            id,
            x,
            y,
        );
        Ok((
            hits.into_iter()
                .map(|hit| Hit {
                    node: Node {
                        context: self.id,
                        id: hit.node,
                    },
                    x: hit.x,
                    y: hit.y,
                })
                .collect(),
            region,
        ))
    }
}
fn validate_text(text: &TextMeasureContext) -> Result<()> {
    if !text.font_size.is_finite()
        || text.font_size <= 0.0
        || !text.line_height.is_finite()
        || text.line_height <= 0.0
        || !text.letter_spacing.is_finite()
        || !(1..=1000).contains(&text.font_weight)
    {
        return Err("Invalid text metrics");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct AtlasInfo {
    pub color: bool,
    pub width: u32,
    pub height: u32,
    pub revision: u32,
    pub x: u32,
    pub y: u32,
    pub update_width: u32,
    pub update_height: u32,
}
impl AtlasInfo {
    pub fn bytes(self) -> usize {
        self.update_width as usize * self.update_height as usize * if self.color { 4 } else { 1 }
    }
}
#[derive(Clone, Copy)]
struct Change {
    revision: u32,
    rect: (u32, u32, u32, u32),
    dimensions: (u32, u32),
}
#[derive(Default)]
struct AtlasHistory {
    revision: u32,
    changes: VecDeque<Change>,
}

/// A renderer owns its atlas cache; display-list vectors retain capacity between frames.
pub struct SceneEncoder {
    renderer: TextRenderer,
    records: Vec<f32>,
    points: Vec<f32>,
    clips: Vec<Clip>,
    count: usize,
    prepared: Option<(u64, u64)>,
    atlases: [AtlasHistory; 2],
}
impl Default for SceneEncoder {
    fn default() -> Self {
        Self::new()
    }
}
impl SceneEncoder {
    pub fn new() -> Self {
        Self {
            renderer: TextRenderer::new(),
            records: Vec::new(),
            points: Vec::new(),
            clips: Vec::new(),
            count: 0,
            prepared: None,
            atlases: Default::default(),
        }
    }
    pub fn prepare(
        &mut self,
        context: &LayoutContext,
        root: Node,
        options: PaintOptions,
    ) -> Result<PaintInfo> {
        self.prepared = None;
        let id = context.prepared_root(root)?;
        if !options.scale.is_finite()
            || options.scale <= 0.0
            || ![
                options.shapes.n,
                options.shapes.threshold,
                options.shapes.radius_full,
            ]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            || !options
                .text_color
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return Err("Invalid paint options");
        }
        self.renderer.begin_atlas_epoch();
        for attempt in 0..2 {
            self.records.clear();
            self.points.clear();
            self.clips.clear();
            let mut glyphs = Glyphs {
                renderer: &mut self.renderer,
                display_scale: options.scale,
                atlas_full: false,
                shapes: options.shapes,
                points: std::mem::take(&mut self.points),
            };
            display_list::append(
                context.tree.as_ref().ok_or("Layout context is disposed")?,
                id,
                (0.0, 0.0),
                1.0,
                options.text_color,
                display_list::IDENTITY,
                &mut self.clips,
                &mut glyphs,
                &mut self.records,
            );
            self.points = std::mem::take(&mut glyphs.points);
            if !glyphs.atlas_full {
                break;
            }
            self.renderer.clear();
            for history in &mut self.atlases {
                history.changes.clear();
            }
            if attempt == 1 {
                self.records.clear();
                return Err("Scene glyphs exceed atlas capacity");
            }
        }
        self.count = self.records.len() / RECORD_FLOATS;
        let base = (self.records.len() / 4) as f32;
        for record in self.records.as_chunks_mut::<RECORD_FLOATS>().0 {
            if record[94] == display_list::SHAPE_POLYGON {
                record[96] += base;
            }
        }
        self.records.extend_from_slice(&self.points);
        self.records.resize(
            self.records.len().div_ceil(RECORD_FLOATS) * RECORD_FLOATS,
            0.0,
        );
        self.prepared = Some((context.id, context.revision));
        Ok(PaintInfo {
            count: self.count,
            floats: self.records.len(),
        })
    }
    pub fn read(&self, context: &LayoutContext, output: &mut [f32]) -> Result<()> {
        if context.is_disposed() {
            return Err("Layout context is disposed");
        }
        if self.prepared != Some((context.id, context.revision)) {
            return Err("Prepare the display list after scene changes");
        }
        if output.len() < self.records.len() {
            return Err("Display-list output is too small");
        }
        output[..self.records.len()].copy_from_slice(&self.records);
        Ok(())
    }
    pub fn atlas_info(&mut self, color: bool, seen: u32) -> Result<Option<AtlasInfo>> {
        let r = &mut self.renderer;
        let (dirty, rect, dimensions) = if color {
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
        let history = &mut self.atlases[color as usize];
        if dirty {
            history.revision = history
                .revision
                .checked_add(1)
                .ok_or("Atlas revision exhausted")?;
            history.changes.push_back(Change {
                revision: history.revision,
                rect: rect.unwrap_or((0, 0, dimensions.0, dimensions.1)),
                dimensions,
            });
            if history.changes.len() > 32 {
                history.changes.pop_front();
            }
            if color {
                r.mark_color_atlas_clean();
            } else {
                r.mark_atlas_clean();
            }
        }
        if seen == history.revision {
            return Ok(None);
        }
        let mut since = history
            .changes
            .iter()
            .filter(|c| c.revision > seen)
            .peekable();
        let mut known = seen > 0
            && since
                .peek()
                .is_some_and(|c| Some(c.revision) == seen.checked_add(1));
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for c in since {
            known &= c.dimensions == dimensions;
            let (x, y, w, h) = c.rect;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + w);
            y1 = y1.max(y + h);
        }
        let (x, y, w, h) = if known {
            (x0, y0, x1 - x0, y1 - y0)
        } else {
            (0, 0, dimensions.0, dimensions.1)
        };
        Ok(Some(AtlasInfo {
            color,
            width: dimensions.0,
            height: dimensions.1,
            revision: history.revision,
            x,
            y,
            update_width: w,
            update_height: h,
        }))
    }
    pub fn read_atlas(
        &mut self,
        color: bool,
        seen: u32,
        output: &mut [u8],
    ) -> Result<Option<AtlasInfo>> {
        let Some(info) = self.atlas_info(color, seen)? else {
            return Ok(None);
        };
        if output.len() < info.bytes() {
            return Err("Atlas output is too small");
        }
        let pixels = if color {
            self.renderer.color_atlas_pixels()
        } else {
            self.renderer.atlas_pixels()
        };
        let bpp = if color { 4 } else { 1 };
        let stride = info.width as usize * bpp;
        let row = info.update_width as usize * bpp;
        for y in 0..info.update_height as usize {
            let start = (info.y as usize + y) * stride + info.x as usize * bpp;
            output[y * row..(y + 1) * row].copy_from_slice(&pixels[start..start + row]);
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blinc_core::{Brush, Color};
    use taffy::prelude::*;
    fn style(w: f32, h: f32) -> Style {
        Style {
            size: Size {
                width: Dimension::length(w),
                height: Dimension::length(h),
            },
            flex_shrink: 0.0,
            ..Default::default()
        }
    }
    #[test]
    fn a_background_image_source_is_noted_when_it_changes() {
        let mut tree = LayoutContext::new();
        let node = tree.create_node(style(10.0, 10.0)).unwrap();
        let image = |source: &str| RenderProps {
            background: Some(Brush::Image(blinc_core::ImageBrush::new(source))),
            ..Default::default()
        };
        let uses = |tree: &mut LayoutContext| {
            tree.take_image_uses()
                .into_iter()
                .map(|(n, s)| (n.raw() == node.raw(), s))
                .collect::<Vec<_>>()
        };
        tree.set_properties(node, image("a.png")).unwrap();
        assert_eq!(uses(&mut tree), [(true, Some("a.png".to_string()))]);
        // The same source again, or another paint field, is no change.
        tree.set_properties(node, image("a.png")).unwrap();
        let mut other = image("a.png");
        other.opacity = 0.5;
        tree.set_properties(node, other).unwrap();
        assert!(uses(&mut tree).is_empty());
        tree.set_properties(node, image("b.png")).unwrap();
        assert_eq!(uses(&mut tree), [(true, Some("b.png".to_string()))]);
        // Another kind of background is no image at all.
        let solid = RenderProps {
            background: Some(Brush::Solid(Color::BLACK)),
            ..Default::default()
        };
        tree.set_properties(node, solid).unwrap();
        assert_eq!(uses(&mut tree), [(true, None)]);
        // Taken, they are not told twice.
        assert!(uses(&mut tree).is_empty());
        // A node that changes several times between takes is listed once, as it ended.
        for source in ["c.png", "d.png", "e.png"] {
            tree.set_properties(node, image(source)).unwrap();
        }
        assert_eq!(uses(&mut tree), [(true, Some("e.png".to_string()))]);
        // One removed before they are taken is not listed at all.
        let other = tree.create_node(style(10.0, 10.0)).unwrap();
        tree.set_properties(node, image("f.png")).unwrap();
        tree.set_properties(other, image("g.png")).unwrap();
        tree.remove(node).unwrap();
        let left = tree.take_image_uses();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0.raw(), other.raw());
        assert!(tree.take_image_uses().is_empty());
    }
    #[test]
    fn a_box_is_in_view_where_it_is_drawn() {
        let mut tree = LayoutContext::new();
        let root = tree
            .create_node(Style {
                flex_direction: FlexDirection::Column,
                ..style(100.0, 100.0)
            })
            .unwrap();
        let near = tree.create_node(style(30.0, 20.0)).unwrap();
        let tall = tree.create_node(style(30.0, 200.0)).unwrap();
        let far = tree.create_node(style(30.0, 20.0)).unwrap();
        let empty = tree.create_node(style(0.0, 0.0)).unwrap();
        tree.set_children(root, &[near, tall, far, empty]).unwrap();
        tree.compute(root, 100.0, 100.0).unwrap();
        assert!(tree.in_view(near).unwrap());
        assert!(!tree.in_view(far).unwrap(), "below the root's edge");
        // A layout animation drawing it back up brings it into view.
        tree.set_visual(far, Some([0.0, -150.0, -1.0, 0.0]))
            .unwrap();
        assert!(tree.in_view(far).unwrap());
        tree.set_properties(
            near,
            RenderProps {
                visible: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!tree.in_view(near).unwrap(), "hidden");
        assert!(
            !tree.in_view(empty).unwrap(),
            "nor is a box of no size below the edge"
        );
    }

    #[test]
    fn a_view_answers_as_in_view_does_sharing_its_walk() {
        let mut tree = LayoutContext::new();
        let column = |w: f32, h: f32| Style {
            flex_direction: FlexDirection::Column,
            ..style(w, h)
        };
        let root = tree.create_node(column(100.0, 100.0)).unwrap();
        let scroller = tree
            .create_node(Style {
                overflow: taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Hidden,
                },
                ..column(50.0, 30.0)
            })
            .unwrap();
        let rows: Vec<_> = (0..4)
            .map(|_| tree.create_node(style(50.0, 20.0)).unwrap())
            .collect();
        tree.set_children(scroller, &rows).unwrap();
        let hidden = tree.create_node(column(30.0, 10.0)).unwrap();
        let under_hidden = tree.create_node(style(10.0, 5.0)).unwrap();
        tree.set_children(hidden, &[under_hidden]).unwrap();
        let gone = tree
            .create_node(Style {
                display: Display::None,
                ..column(30.0, 10.0)
            })
            .unwrap();
        let under_gone = tree.create_node(style(10.0, 5.0)).unwrap();
        tree.set_children(gone, &[under_gone]).unwrap();
        let far = tree.create_node(style(30.0, 200.0)).unwrap();
        let moved = tree.create_node(style(30.0, 10.0)).unwrap();
        tree.set_children(root, &[scroller, hidden, gone, far, moved])
            .unwrap();
        tree.compute(root, 100.0, 100.0).unwrap();
        tree.set_scroll(
            scroller,
            Some(crate::tree::Scroll {
                x: 0.0,
                y: 25.0,
                thumb: [0.0; 4],
            }),
        )
        .unwrap();
        tree.set_properties(
            hidden,
            RenderProps {
                visible: false,
                ..Default::default()
            },
        )
        .unwrap();
        tree.set_visual(moved, Some([0.0, -200.0, -1.0, 0.0]))
            .unwrap();
        let all = [
            vec![
                root,
                scroller,
                hidden,
                under_hidden,
                gone,
                under_gone,
                far,
                moved,
            ],
            rows.clone(),
        ]
        .concat();
        // Asked leaves first and in reverse, so ancestors are reached through their children.
        for order in [all.clone(), all.iter().rev().copied().collect()] {
            let mut view = tree.view().unwrap();
            for &n in &order {
                assert_eq!(view.in_view(n).unwrap(), tree.in_view(n).unwrap(), "{n:?}");
            }
        }
        let mut view = tree.view().unwrap();
        let seen: Vec<bool> = rows.iter().map(|&r| view.in_view(r).unwrap()).collect();
        assert_eq!(
            seen,
            [false, true, true, false],
            "scrolled by 25 in a 30 tall clip"
        );
        assert!(!view.in_view(under_hidden).unwrap());
        assert!(!view.in_view(under_gone).unwrap());
        assert!(view.in_view(moved).unwrap(), "drawn back into view");
    }

    #[test]
    fn records_hits_visuals_and_stale_buffers() {
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(100.0, 100.0)).unwrap();
        let child = tree.create_node(style(30.0, 20.0)).unwrap();
        tree.set_children(root, &[child]).unwrap();
        tree.set_properties(
            child,
            RenderProps {
                background: Some(Brush::Solid(Color::from_hex(0xff0000))),
                ..Default::default()
            },
        )
        .unwrap();
        let mut encoder = SceneEncoder::new();
        assert!(
            encoder
                .prepare(&tree, root, PaintOptions::default())
                .is_err()
        );
        tree.compute(root, 100.0, 100.0).unwrap();
        let info = encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert_eq!(info.count, 1);
        let mut data = vec![0.0; info.floats];
        encoder.read(&tree, &mut data).unwrap();
        assert_eq!(&data[..4], &[0.0, 0.0, 30.0, 20.0]);
        assert_eq!(&data[8..12], &[1.0, 0.0, 0.0, 1.0]);
        assert_eq!(tree.hit_test(root, 5.0, 5.0).unwrap()[0].node, child);
        tree.set_visual(child, Some([40.0, 20.0, 35.0, 25.0]))
            .unwrap();
        assert!(encoder.read(&tree, &mut data).is_err());
        assert_eq!(tree.hit_test(root, 45.0, 25.0).unwrap()[0].node, child);
        assert_eq!(tree.hit_test(root, 5.0, 5.0).unwrap()[0].node, root);
        encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        encoder.read(&tree, &mut data).unwrap();
        assert_eq!(&data[..4], &[40.0, 20.0, 35.0, 25.0]);
        tree.set_pointer_events(child, false).unwrap();
        assert_eq!(tree.hit_test(root, 45.0, 25.0).unwrap()[0].node, root);
        let other = LayoutContext::new();
        assert!(encoder.read(&other, &mut data).is_err());
        tree.dispose();
        assert!(tree.properties(child).is_err());
        assert!(encoder.read(&tree, &mut data).is_err());
    }
    #[test]
    fn a_decoration_stacks_with_an_ancestors_and_an_explicit_none_does_not_undo_it() {
        use crate::css::text_decoration::{Style as DecorationStyle, TextDecoration, UNDERLINE};
        use crate::display_list::{PRIM_RECT, RECORD_FLOATS};
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(300.0, 100.0)).unwrap();
        let child = tree
            .create_text(
                Style::default(),
                TextMeasureContext {
                    content: "WW".into(),
                    font_size: 16.0,
                    line_height: 1.2,
                    letter_spacing: 0.0,
                    wrap: false,
                    font_name: None,
                    generic_font: blinc_layout::div::GenericFont::SansSerif,
                    font_weight: 400,
                    italic: false,
                },
            )
            .unwrap();
        tree.set_children(root, &[child]).unwrap();
        tree.compute(root, 300.0, 100.0).unwrap();
        let underline = TextDecoration {
            lines: UNDERLINE,
            style: DecorationStyle::Solid,
            color: None,
            thickness: None,
            offset: None,
        };
        let mut encoder = SceneEncoder::new();
        let bands = |tree: &LayoutContext, encoder: &mut SceneEncoder| -> usize {
            encoder
                .prepare(tree, root, PaintOptions::default())
                .unwrap();
            encoder
                .records
                .chunks(RECORD_FLOATS)
                .filter(|r| r[44] as u32 == PRIM_RECT as u32)
                .count()
        };
        tree.set_text_decoration(root, Some(underline)).unwrap();
        assert_eq!(
            bands(&tree, &mut encoder),
            1,
            "the root's underline draws once, across its only child"
        );
        // The child's own `none`: it draws no line of its own, and does not undo the root's.
        let none = TextDecoration {
            lines: 0,
            ..underline
        };
        tree.set_text_decoration(child, Some(none)).unwrap();
        assert_eq!(
            bands(&tree, &mut encoder),
            1,
            "a child's own none neither adds a line nor removes its ancestor's"
        );
        // The child decorated too: both bands draw, the child's innermost.
        tree.set_text_decoration(child, Some(underline)).unwrap();
        assert_eq!(
            bands(&tree, &mut encoder),
            2,
            "a child's own line stacks with its ancestor's rather than replacing it"
        );
    }
    #[test]
    fn z_index_stacks_siblings_and_the_hit_test_follows() {
        use crate::context::{PropValue, Z_INDEX};
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(100.0, 100.0)).unwrap();
        // Three boxes in one place, in tree order red, green, blue.
        let boxes: Vec<_> = [0xff0000u32, 0x00ff00, 0x0000ff]
            .iter()
            .map(|&hue| {
                let node = tree
                    .create_node(Style {
                        position: Position::Absolute,
                        ..style(50.0, 50.0)
                    })
                    .unwrap();
                tree.set_properties(
                    node,
                    RenderProps {
                        background: Some(Brush::Solid(Color::from_hex(hue))),
                        ..Default::default()
                    },
                )
                .unwrap();
                node
            })
            .collect();
        tree.set_children(root, &boxes).unwrap();
        tree.compute(root, 100.0, 100.0).unwrap();
        // Which of red (r), green (g) and blue (b) is drawn first to last.
        let painted = |tree: &mut LayoutContext| -> String {
            tree.compute(root, 100.0, 100.0).unwrap();
            let mut encoder = SceneEncoder::new();
            let info = encoder
                .prepare(tree, root, PaintOptions::default())
                .unwrap();
            let mut data = vec![0.0; info.floats];
            encoder.read(tree, &mut data).unwrap();
            data.chunks(RECORD_FLOATS)
                .map(|r| match (r[8], r[9], r[10]) {
                    (1.0, 0.0, 0.0) => 'r',
                    (0.0, 1.0, 0.0) => 'g',
                    _ => 'b',
                })
                .collect()
        };
        assert_eq!(painted(&mut tree), "rgb", "tree order with no z-index");
        let z = |tree: &mut LayoutContext, node: Node, v: i32| {
            tree.apply(&[(node, Z_INDEX, PropValue::Enum(v))]).unwrap();
            tree.compute(root, 100.0, 100.0).unwrap();
        };
        z(&mut tree, boxes[0], 1);
        assert_eq!(
            painted(&mut tree),
            "gbr",
            "a positive z-index goes to the top"
        );
        assert_eq!(tree.hit_test(root, 10.0, 10.0).unwrap()[0].node, boxes[0]);
        z(&mut tree, boxes[2], -1);
        assert_eq!(
            painted(&mut tree),
            "bgr",
            "a negative one goes under the rest"
        );
        z(&mut tree, boxes[1], 2);
        assert_eq!(painted(&mut tree), "brg", "higher is over lower");
        assert_eq!(tree.hit_test(root, 10.0, 10.0).unwrap()[0].node, boxes[1]);
        // Equal z-indexes keep tree order; zero is with the positioned ones, above the others.
        z(&mut tree, boxes[1], 1);
        assert_eq!(painted(&mut tree), "brg");
        tree.apply(&[(boxes[0], Z_INDEX, PropValue::Unset)])
            .unwrap();
        tree.apply(&[(boxes[1], Z_INDEX, PropValue::Unset)])
            .unwrap();
        tree.apply(&[(boxes[2], Z_INDEX, PropValue::Unset)])
            .unwrap();
        assert_eq!(
            painted(&mut tree),
            "rgb",
            "auto puts them back in tree order"
        );
        assert!(
            tree.apply(&[(boxes[0], Z_INDEX, PropValue::Text(None))])
                .is_err()
        );
    }

    #[test]
    fn a_sticky_header_is_held_in_view_and_leaves_with_its_section() {
        use crate::context::{INSET_TOP, POSITION, PropValue, STICKY};
        let mut tree = LayoutContext::new();
        let column = |w: f32, h: f32| Style {
            flex_direction: FlexDirection::Column,
            ..style(w, h)
        };
        let scroller = tree
            .create_node(Style {
                overflow: taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Hidden,
                },
                ..column(100.0, 50.0)
            })
            .unwrap();
        let mut headers = Vec::new();
        let mut sections = Vec::new();
        for hue in [0xff0000, 0x0000ff] {
            let header = tree.create_node(style(100.0, 10.0)).unwrap();
            let body = tree.create_node(style(100.0, 110.0)).unwrap();
            let section = tree.create_node(column(100.0, 120.0)).unwrap();
            tree.set_children(section, &[header, body]).unwrap();
            tree.set_properties(
                header,
                RenderProps {
                    background: Some(Brush::Solid(Color::from_hex(hue))),
                    ..Default::default()
                },
            )
            .unwrap();
            tree.apply(&[
                (header, INSET_TOP, PropValue::Number(0.0)),
                (header, POSITION, PropValue::Enum(STICKY)),
            ])
            .unwrap();
            headers.push(header);
            sections.push(section);
        }
        tree.set_children(scroller, &sections).unwrap();
        tree.compute(scroller, 100.0, 50.0).unwrap();
        // Sticky is laid out where it would be without its inset, not shifted by it.
        let mut at = [0.0; 4];
        tree.read_bounds(&[headers[1]], &mut at).unwrap();
        assert_eq!(&at[..2], &[0.0, 120.0], "{at:?}");
        const GONE: f32 = -999.0;
        // The red and blue headers' drawn tops, at a scroll offset.
        let drawn = |tree: &mut LayoutContext, scroll: f32| -> [f32; 2] {
            tree.set_scroll(
                scroller,
                Some(Scroll {
                    x: 0.0,
                    y: scroll,
                    thumb: [0.0; 4],
                }),
            )
            .unwrap();
            let mut encoder = SceneEncoder::new();
            let info = encoder
                .prepare(tree, scroller, PaintOptions::default())
                .unwrap();
            let mut data = vec![0.0; info.floats];
            encoder.read(tree, &mut data).unwrap();
            // A header out of the view is not drawn at all.
            let mut tops = [GONE; 2];
            for r in data.chunks(RECORD_FLOATS) {
                // The fill's colour is in the third row.
                if r[8] == 1.0 && r[10] == 0.0 {
                    tops[0] = r[1];
                } else if r[10] == 1.0 && r[8] == 0.0 {
                    tops[1] = r[1];
                }
            }
            tops
        };
        // Unscrolled, the first is where layout put it, and the second is below the view.
        assert_eq!(drawn(&mut tree, 0.0), [0.0, GONE]);
        // Scrolled into the first section, its header stays at the top of the view.
        assert_eq!(drawn(&mut tree, 40.0), [0.0, GONE]);
        // At the end of the first section, its header goes up with it, while the second arrives.
        assert_eq!(drawn(&mut tree, 115.0), [-5.0, 5.0]);
        // Past it, the first is gone and the second is held.
        assert_eq!(drawn(&mut tree, 150.0), [GONE, 0.0]);
        // A held header is drawn over the body that follows it, and is what is hit over it.
        drawn(&mut tree, 40.0);
        assert_eq!(
            tree.hit_test(scroller, 5.0, 5.0).unwrap()[0].node,
            headers[0]
        );
        // A node made not sticky is laid out and drawn as a relative one is, offset by its inset.
        tree.apply(&[(headers[0], POSITION, PropValue::Enum(0))])
            .unwrap();
        tree.compute(scroller, 100.0, 50.0).unwrap();
        assert_eq!(
            drawn(&mut tree, 40.0)[0],
            GONE,
            "scrolled out of the view with its section"
        );
    }
    #[test]
    fn text_that_overflows_its_container_ends_with_an_ellipsis_where_asked() {
        use crate::display_list::RECORD_FLOATS;
        let mut tree = LayoutContext::new();
        let container = tree
            .create_node(Style {
                overflow: taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Hidden,
                },
                padding: taffy::Rect {
                    left: taffy::LengthPercentage::length(4.0),
                    right: taffy::LengthPercentage::length(6.0),
                    top: taffy::LengthPercentage::length(0.0),
                    bottom: taffy::LengthPercentage::length(0.0),
                },
                ..style(100.0, 30.0)
            })
            .unwrap();
        let text = tree
            .create_text(
                Style::default(),
                TextMeasureContext {
                    content: "WWWWWWWWWWWWWWWW".into(),
                    font_size: 16.0,
                    line_height: 1.2,
                    letter_spacing: 0.0,
                    wrap: false,
                    font_name: None,
                    generic_font: blinc_layout::div::GenericFont::SansSerif,
                    font_weight: 400,
                    italic: false,
                },
            )
            .unwrap();
        // The text is one level down, in a box that sizes to it.
        let wrapper = tree.create_node(Style::default()).unwrap();
        tree.set_children(wrapper, &[text]).unwrap();
        tree.set_children(container, &[wrapper]).unwrap();
        tree.compute(container, 100.0, 30.0).unwrap();
        // The glyphs' count and right-most edge.
        let glyphs = |tree: &LayoutContext| -> (usize, f32) {
            let mut encoder = SceneEncoder::new();
            let info = encoder
                .prepare(tree, container, PaintOptions::default())
                .unwrap();
            let mut data = vec![0.0; info.floats];
            encoder.read(tree, &mut data).unwrap();
            data.chunks(RECORD_FLOATS)
                .fold((0, 0.0f32), |(n, edge), r| (n + 1, edge.max(r[0] + r[2])))
        };
        let (all, overflow) = glyphs(&tree);
        assert!(overflow > 100.0, "the text overflows its box: {overflow}");
        tree.set_text_overflow(container, Some(false)).unwrap();
        assert_eq!(glyphs(&tree), (all, overflow), "clip leaves it as it is");
        tree.set_text_overflow(container, Some(true)).unwrap();
        let (cut, edge) = glyphs(&tree);
        // Within the container's content box, 4 in from the left and 6 from the right, give or take
        // the ink that reaches past a glyph's advance.
        assert!(edge <= 96.0, "ends within the content box: {edge}");
        assert!(edge >= 78.0, "and uses most of it: {edge}");
        assert!(cut < all, "with fewer glyphs: {cut} of {all}");
        // A text that fits is untouched.
        tree.set_text_overflow(container, None).unwrap();
        let mut context = tree.text(text).unwrap();
        context.content = "WW".into();
        tree.set_text(text, context).unwrap();
        tree.compute(container, 100.0, 30.0).unwrap();
        let (two, _) = glyphs(&tree);
        tree.set_text_overflow(container, Some(true)).unwrap();
        assert_eq!(glyphs(&tree).0, two, "a line that fits has no ellipsis");
        assert!(tree.set_text_overflow(container, None).is_ok());
    }

    #[test]
    fn a_line_clamp_holds_text_to_that_many_lines_with_an_ellipsis() {
        use crate::display_list::RECORD_FLOATS;
        let mut tree = LayoutContext::new();
        let container = tree
            .create_node(Style {
                flex_direction: FlexDirection::Column,
                ..style(100.0, 200.0)
            })
            .unwrap();
        let text = tree
            .create_text(
                Style::default(),
                TextMeasureContext {
                    content: "one two three four five six seven eight nine ten eleven twelve"
                        .into(),
                    font_size: 16.0,
                    line_height: 1.2,
                    letter_spacing: 0.0,
                    wrap: true,
                    font_name: None,
                    generic_font: blinc_layout::div::GenericFont::SansSerif,
                    font_weight: 400,
                    italic: false,
                },
            )
            .unwrap();
        tree.set_children(container, &[text]).unwrap();
        tree.compute(container, 100.0, 200.0).unwrap();
        let measure = |tree: &LayoutContext| -> (f32, usize, usize) {
            let mut at = [0.0; 4];
            tree.read_bounds(&[text], &mut at).unwrap();
            let mut encoder = SceneEncoder::new();
            let info = encoder
                .prepare(tree, container, PaintOptions::default())
                .unwrap();
            let mut data = vec![0.0; info.floats];
            encoder.read(tree, &mut data).unwrap();
            let mut tops: Vec<i32> = data
                .chunks(RECORD_FLOATS)
                .map(|r| (r[1] / 19.2).floor() as i32)
                .collect();
            let glyphs = tops.len();
            tops.dedup();
            (at[3], tops.len(), glyphs)
        };
        let (natural, lines, glyphs) = measure(&tree);
        assert!(lines >= 5, "it wraps to several lines: {lines}");
        tree.set_line_clamp(container, Some(2)).unwrap();
        tree.compute(container, 100.0, 200.0).unwrap();
        let (held, shown, cut) = measure(&tree);
        assert!(held < natural, "the box is shorter: {held} for {natural}");
        assert!((held - 2.0 * 19.2).abs() < 1.5, "two lines of 19.2: {held}");
        assert_eq!(shown, 2, "two lines are drawn");
        assert!(cut < glyphs, "of fewer glyphs: {cut} of {glyphs}");
        // A larger clamp than the text has changes nothing; none lets go of the height.
        tree.set_line_clamp(container, Some(40)).unwrap();
        tree.compute(container, 100.0, 200.0).unwrap();
        assert_eq!(measure(&tree), (natural, lines, glyphs));
        tree.set_line_clamp(container, Some(2)).unwrap();
        tree.compute(container, 100.0, 200.0).unwrap();
        tree.set_line_clamp(container, None).unwrap();
        tree.compute(container, 100.0, 200.0).unwrap();
        assert_eq!(
            measure(&tree),
            (natural, lines, glyphs),
            "taken away, as it was"
        );
    }

    #[test]
    fn text_shadows_draw_under_the_text_last_first_and_inherit() {
        use crate::display_list::{PRIM_LAYER, PRIM_LAYER_BEGIN, PRIM_TEXT, RECORD_FLOATS};
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(300.0, 100.0)).unwrap();
        let text = tree
            .create_text(
                Style::default(),
                TextMeasureContext {
                    content: "WW".into(),
                    font_size: 16.0,
                    line_height: 1.2,
                    letter_spacing: 0.0,
                    wrap: false,
                    font_name: None,
                    generic_font: blinc_layout::div::GenericFont::SansSerif,
                    font_weight: 400,
                    italic: false,
                },
            )
            .unwrap();
        tree.set_children(root, &[text]).unwrap();
        tree.compute(root, 300.0, 100.0).unwrap();
        let shadow = |dx: f32, blur: f32, r: f32| blinc_core::Shadow {
            offset_x: dx,
            offset_y: 2.0,
            blur,
            spread: 0.0,
            color: blinc_core::Color::rgba(r, 0.0, 0.0, 1.0),
        };
        let mut encoder = SceneEncoder::new();
        let mut kinds = |tree: &LayoutContext| -> Vec<(u32, f32)> {
            encoder
                .prepare(tree, root, PaintOptions::default())
                .unwrap();
            encoder
                .records
                .chunks(RECORD_FLOATS)
                .map(|r| (r[44] as u32, r[8]))
                .collect()
        };
        // On the root, so the text inherits them: a hard one on top, a blurred one under it.
        tree.set_text_shadow(
            root,
            Some(vec![shadow(2.0, 0.0, 0.5), shadow(4.0, 6.0, 1.0)]),
        )
        .unwrap();
        let drawn = kinds(&tree);
        let order: Vec<u32> = drawn.iter().map(|k| k.0).collect();
        assert_eq!(
            order,
            [
                PRIM_LAYER_BEGIN,
                PRIM_TEXT,
                PRIM_TEXT,
                PRIM_LAYER,
                PRIM_TEXT,
                PRIM_TEXT,
                PRIM_TEXT,
                PRIM_TEXT
            ]
            .map(|k| k as u32),
            "the blurred shadow in a layer, then the hard one, then the text"
        );
        // The shadows' glyphs in their own colours, the text's in its.
        assert_eq!((drawn[1].1, drawn[4].1), (1.0, 0.5));
        assert!(drawn[6].1 != 1.0 && drawn[6].1 != 0.5);
        // `none` on the text stops what the root sets.
        tree.set_text_shadow(text, Some(Vec::new())).unwrap();
        assert_eq!(kinds(&tree).len(), 2);
        // Unset there, the root's reach it again; unset on the root, none.
        tree.set_text_shadow(text, None).unwrap();
        assert_eq!(kinds(&tree).len(), 8);
        tree.set_text_shadow(root, None).unwrap();
        assert_eq!(kinds(&tree).len(), 2);
        assert!(
            tree.set_text_shadow(root, Some(vec![shadow(f32::NAN, 0.0, 1.0)]))
                .is_err()
        );
    }

    #[test]
    fn text_out_of_its_clip_is_not_shaped_until_it_scrolls_into_view() {
        let mut tree = LayoutContext::new();
        let column = |w: f32, h: f32| Style {
            flex_direction: FlexDirection::Column,
            ..style(w, h)
        };
        let root = tree.create_node(column(300.0, 100.0)).unwrap();
        let scroller = tree
            .create_node(Style {
                overflow: taffy::Point {
                    x: taffy::Overflow::Hidden,
                    y: taffy::Overflow::Hidden,
                },
                flex_shrink: 0.0,
                ..column(300.0, 40.0)
            })
            .unwrap();
        let text = |content: &str| TextMeasureContext {
            content: content.into(),
            font_size: 16.0,
            line_height: 1.2,
            letter_spacing: 0.0,
            wrap: false,
            font_name: None,
            generic_font: blinc_layout::div::GenericFont::SansSerif,
            font_weight: 400,
            italic: false,
        };
        let seen = tree.create_text(Style::default(), text("WWWW")).unwrap();
        let gap = tree
            .create_node(Style {
                flex_shrink: 0.0,
                ..style(10.0, 400.0)
            })
            .unwrap();
        let below = tree.create_text(Style::default(), text("MMMM")).unwrap();
        tree.set_children(scroller, &[seen, gap, below]).unwrap();
        tree.set_children(root, &[scroller]).unwrap();
        tree.compute(root, 300.0, 100.0).unwrap();
        let mut encoder = SceneEncoder::new();
        let info = encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert_eq!(info.count, 4, "the text in view, one record a glyph");
        let atlas = encoder.atlas_info(false, 0).unwrap().unwrap();
        // Again, as it was: nothing new to rasterize.
        encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert!(encoder.atlas_info(false, atlas.revision).unwrap().is_none());
        // Scrolled to the text below, its glyphs are rasterized for the first time.
        tree.set_scroll(
            scroller,
            Some(crate::tree::Scroll {
                x: 0.0,
                y: 400.0,
                thumb: [0.0; 4],
            }),
        )
        .unwrap();
        let info = encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert_eq!(info.count, 4, "the text scrolled to, and not the other");
        assert!(
            encoder.atlas_info(false, atlas.revision).unwrap().is_some(),
            "the text below was not shaped while out of view"
        );
    }

    #[test]
    fn measured_text_and_incremental_atlases() {
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(500.0, 100.0)).unwrap();
        let text = TextMeasureContext {
            content: "WWWW".into(),
            font_size: 24.0,
            line_height: 1.2,
            letter_spacing: 0.0,
            wrap: false,
            font_name: None,
            generic_font: blinc_layout::div::GenericFont::SansSerif,
            font_weight: 400,
            italic: false,
        };
        let label = tree.create_text(Style::default(), text).unwrap();
        tree.set_children(root, &[label]).unwrap();
        tree.compute(root, 500.0, 100.0).unwrap();
        let mut first = [0.0; 4];
        tree.read_bounds(&[label], &mut first).unwrap();
        let mut encoder = SceneEncoder::new();
        let info = encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert_eq!(info.count, 4);
        let atlas = encoder.atlas_info(false, 0).unwrap().unwrap();
        let mut bytes = vec![0; atlas.bytes()];
        encoder.read_atlas(false, 0, &mut bytes).unwrap();
        assert!(bytes.iter().any(|b| *b > 0));
        assert!(encoder.atlas_info(false, atlas.revision).unwrap().is_none());
        let mut text = tree.text(label).unwrap();
        text.content = "iiii".into();
        tree.set_text(label, text).unwrap();
        tree.compute(root, 500.0, 100.0).unwrap();
        let mut second = [0.0; 4];
        tree.read_bounds(&[label], &mut second).unwrap();
        assert!(
            first[2] > second[2] * 1.5,
            "real proportional font widths: {first:?} {second:?}"
        );
        encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        let update = encoder.atlas_info(false, atlas.revision).unwrap().unwrap();
        assert!(update.revision > atlas.revision);
        assert!(update.bytes() < atlas.bytes());
        assert!(encoder.read_atlas(false, atlas.revision, &mut []).is_err());
        let mut bytes = vec![0; update.bytes()];
        encoder
            .read_atlas(false, atlas.revision, &mut bytes)
            .unwrap();
        assert!(bytes.iter().any(|b| *b > 0));
    }
    #[test]
    fn text_spacing_matches_measured_layout() {
        let mut tree = LayoutContext::new();
        let root = tree.create_node(style(500.0, 100.0)).unwrap();
        let text = TextMeasureContext {
            content: "MMMM".into(),
            font_size: 24.0,
            line_height: 1.2,
            letter_spacing: 0.0,
            wrap: false,
            font_name: None,
            generic_font: blinc_layout::div::GenericFont::SansSerif,
            font_weight: 400,
            italic: false,
        };
        let label = tree.create_text(Style::default(), text).unwrap();
        tree.set_children(root, &[label]).unwrap();
        tree.compute(root, 500.0, 100.0).unwrap();
        let mut encoder = SceneEncoder::new();
        let info = encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        assert_eq!(info.count, 4);
        let mut before = vec![0.0; info.floats];
        encoder.read(&tree, &mut before).unwrap();
        let mut bounds_before = [0.0; 4];
        tree.read_bounds(&[label], &mut bounds_before).unwrap();
        let mut text = tree.text(label).unwrap();
        text.letter_spacing = 10.0;
        tree.set_text(label, text).unwrap();
        tree.compute(root, 500.0, 100.0).unwrap();
        encoder
            .prepare(&tree, root, PaintOptions::default())
            .unwrap();
        let mut after = vec![0.0; info.floats];
        encoder.read(&tree, &mut after).unwrap();
        let mut bounds_after = [0.0; 4];
        tree.read_bounds(&[label], &mut bounds_after).unwrap();
        assert!(bounds_after[2] > bounds_before[2] + 25.0);
        for i in 1..4 {
            let delta = after[i * RECORD_FLOATS] - before[i * RECORD_FLOATS];
            assert!((delta - 10.0 * i as f32).abs() <= 1.0, "glyph {i}: {delta}");
        }
    }
    #[test]
    fn disposed_scene_rejects_access_without_changing_output() {
        let mut ctx = LayoutContext::new();
        let root = ctx.create_node(style(100.0, 100.0)).unwrap();
        let text = TextMeasureContext {
            content: "Disposed".into(),
            font_size: 16.0,
            line_height: 1.2,
            letter_spacing: 0.0,
            wrap: false,
            font_name: None,
            generic_font: blinc_layout::div::GenericFont::SansSerif,
            font_weight: 400,
            italic: false,
        };
        let label = ctx.create_text(Style::default(), text.clone()).unwrap();
        ctx.set_children(root, &[label]).unwrap();
        ctx.compute(root, 100.0, 100.0).unwrap();
        let mut encoder = SceneEncoder::new();
        let info = encoder
            .prepare(&ctx, root, PaintOptions::default())
            .unwrap();
        let mut out = vec![9.0; info.floats];
        ctx.dispose();
        assert_eq!(
            ctx.properties(root).err(),
            Some("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_properties(root, RenderProps::default()),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.create_text(Style::default(), text.clone()).unwrap_err(),
            "Layout context is disposed"
        );
        assert_eq!(ctx.text(label).err(), Some("Layout context is disposed"));
        assert_eq!(ctx.set_text(label, text), Err("Layout context is disposed"));
        assert_eq!(
            ctx.set_visual(root, None),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_scroll(root, None),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_pointer_events(root, true),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_resource(root, None, false),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_glass_effects(root, None),
            Err("Layout context is disposed")
        );
        assert_eq!(
            ctx.set_backdrop_filters(root, None),
            Err("Layout context is disposed")
        );
        assert_eq!(ctx.set_notch(root, None), Err("Layout context is disposed"));
        assert_eq!(
            ctx.hit_test(root, 0.0, 0.0).err(),
            Some("Layout context is disposed")
        );
        assert_eq!(
            encoder.read(&ctx, &mut out),
            Err("Layout context is disposed")
        );
        assert_eq!(
            encoder.prepare(&ctx, root, PaintOptions::default()).err(),
            Some("Layout context is disposed")
        );
        assert!(out.iter().all(|v| *v == 9.0));
    }
}
