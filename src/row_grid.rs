//! Shared native row-grid regression fixtures. Slots remain adapter-owned.
#[cfg(test)]
mod tests {
    use crate::context::{LayoutContext, Node};
    #[cfg(feature = "scene")]
    use taffy::Overflow;
    use taffy::prelude::*;

    pub(crate) fn fixture() -> (LayoutContext, Node, [Node; 3], [Node; 5]) {
        let mut ctx = LayoutContext::new();
        let group = ctx
            .create_node(Style {
                size: Size {
                    width: Dimension::length(100.),
                    height: Dimension::auto(),
                },
                flex_direction: FlexDirection::Column,
                ..Default::default()
            })
            .unwrap();
        let rows = std::array::from_fn(|_| {
            ctx.create_node(Style {
                display: Display::Grid,
                grid_template_columns: crate::grid::template("40px 60px").unwrap(),
                ..Default::default()
            })
            .unwrap()
        });
        let cells = std::array::from_fn(|i| {
            ctx.create_node(Style {
                min_size: Size {
                    width: LengthPercentageAuto::auto(),
                    height: LengthPercentageAuto::length(if i == 0 { 80. } else { 20. }),
                },
                grid_column: crate::grid::line_pair(if i == 0 || i == 3 { "1" } else { "2" })
                    .unwrap(),
                grid_row: crate::grid::line_pair(if i == 0 { "1 / span 2" } else { "1" }).unwrap(),
                ..Default::default()
            })
            .unwrap()
        });
        ctx.set_children(rows[0], &cells[0..2]).unwrap();
        ctx.set_children(rows[1], &cells[2..3]).unwrap();
        ctx.set_children(rows[2], &cells[3..5]).unwrap();
        ctx.set_children(group, &rows).unwrap();
        (ctx, group, rows, cells)
    }

    fn bounds(ctx: &LayoutContext, node: Node) -> [f32; 4] {
        let mut out = [0.; 4];
        ctx.read_bounds(&[node], &mut out).unwrap();
        out
    }

    #[test]
    fn shared_rows_size_spans_and_preserve_authored_ancestry() {
        let (mut ctx, group, rows, cells) = fixture();
        ctx.set_grid_rows(group, &rows).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        let a = bounds(&ctx, rows[0]);
        let b = bounds(&ctx, rows[1]);
        let c = bounds(&ctx, cells[0]);
        assert_eq!(c, [0., 0., 40., 80.]);
        assert_eq!(a[3] + b[3], 80.);
        assert_eq!(bounds(&ctx, cells[2])[0], 40.);
        assert_eq!(bounds(&ctx, cells[2])[1], b[1]);
        assert_eq!(ctx.parent(cells[0]).unwrap(), Some(rows[0]));
        assert_eq!(ctx.children(rows[0]).unwrap(), cells[0..2]);
        assert_eq!(ctx.children(group).unwrap(), rows);
        ctx.compute(group, 100., 200.).unwrap();
        assert_eq!(
            bounds(&ctx, cells[0]),
            c,
            "cached projection keeps rebased coordinates"
        );
        ctx.set_grid_rows(group, &[]).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        assert!(
            bounds(&ctx, group)[3] > 100.,
            "reset restores independent row grids"
        );
    }

    #[test]
    fn invalid_and_stale_row_projections_cannot_reparent_or_retain_nodes() {
        let (mut ctx, group, rows, cells) = fixture();
        assert!(ctx.set_grid_rows(group, &[rows[0], rows[0]]).is_err());
        assert!(ctx.set_grid_rows(group, &cells).is_err());
        let mut other = LayoutContext::new();
        let foreign = other.create_node(Style::default()).unwrap();
        assert!(ctx.set_grid_rows(group, &[foreign]).is_err());
        ctx.set_grid_rows(group, &rows).unwrap();
        ctx.detach(rows[1]).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        ctx.remove(group).unwrap();
        assert_eq!(
            ctx.len().unwrap(),
            2,
            "detached row and its cell survive, without projection ownership"
        );
    }

    #[test]
    fn hidden_rows_collapse_their_span_tracks() {
        let (mut ctx, group, rows, cells) = fixture();
        let mut hidden = ctx.style(rows[1]).unwrap();
        hidden.display = Display::None;
        ctx.set_style(rows[1], hidden).unwrap();
        ctx.set_grid_rows(group, &rows).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        assert_eq!(bounds(&ctx, rows[1])[3], 0.);
        assert_eq!(bounds(&ctx, cells[0])[3], bounds(&ctx, rows[0])[3]);
        assert_eq!(bounds(&ctx, rows[2])[1], 80.);
    }

    #[test]
    fn hiding_the_registered_group_keeps_its_entire_subtree_hidden() {
        let (mut ctx, group, rows, cells) = fixture();
        ctx.set_grid_rows(group, &rows).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        let original = bounds(&ctx, cells[0]);
        let mut style = ctx.style(group).unwrap();
        style.display = Display::None;
        ctx.set_style(group, style.clone()).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        assert_eq!(bounds(&ctx, group), [0.; 4]);
        for node in rows.into_iter().chain(cells) {
            assert_eq!(bounds(&ctx, node), [0.; 4]);
        }
        style.display = Display::Flex;
        ctx.set_style(group, style).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        assert_eq!(bounds(&ctx, cells[0]), original);
    }

    #[cfg(feature = "scene")]
    #[test]
    fn row_backgrounds_stay_below_spans_and_hits_keep_the_original_row_path() {
        use crate::display_list::RECORD_FLOATS;
        use crate::scene::{PaintOptions, SceneEncoder};
        use blinc_core::{Brush, Color};
        use blinc_layout::element::RenderProps;
        let (mut ctx, group, rows, cells) = fixture();
        ctx.set_grid_rows(group, &rows).unwrap();
        for row in rows {
            ctx.set_properties(
                row,
                RenderProps {
                    background: Some(Brush::Solid(Color::GREEN)),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        ctx.set_properties(
            cells[0],
            RenderProps {
                background: Some(Brush::Solid(Color::RED)),
                ..Default::default()
            },
        )
        .unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        let mut encoder = SceneEncoder::new();
        let info = encoder
            .prepare(&ctx, group, PaintOptions::default())
            .unwrap();
        let mut records = vec![0.; info.floats];
        encoder.read(&ctx, &mut records).unwrap();
        let colors: Vec<_> = records
            .chunks_exact(RECORD_FLOATS)
            .map(|r| &r[8..12])
            .collect();
        assert_eq!(colors.last().copied(), Some(&[1., 0., 0., 1.][..]));
        assert_eq!(
            ctx.hit_test(group, 10., 60.)
                .unwrap()
                .iter()
                .map(|h| h.node)
                .collect::<Vec<_>>(),
            [cells[0], rows[0], group]
        );
        let (hits, _) = ctx.hit_test_region(group, 10., 60.).unwrap();
        assert_eq!(hits[0].node, cells[0]);
        let mut clipped = ctx.style(rows[0]).unwrap();
        clipped.overflow.y = Overflow::Hidden;
        ctx.set_style(rows[0], clipped).unwrap();
        ctx.compute(group, 100., 200.).unwrap();
        assert_ne!(ctx.hit_test(group, 10., 60.).unwrap()[0].node, cells[0]);
    }
}
