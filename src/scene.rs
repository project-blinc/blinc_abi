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
        self.tree
            .as_mut()
            .ok_or("Layout context is disposed")?
            .props
            .insert(id, props);
        self.revision += 1;
        Ok(())
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
