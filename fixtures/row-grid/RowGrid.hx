import ashui.core.render.Offscreen;
import ashui.css.Css;
import ashui.css.Identity;
import ashui.layout.LayoutTree;
import ashui.ui.Div;
import gpu.GpuTextureViewDescriptor;

@:hlNative("blinc_abi")
extern class RowGridNative {
    static function blinc_tree_set_grid_rows(tree:hl.Abstract<"blinc_tree">, group:haxe.Int64,
        rows:hl.Bytes, length:Int):Bool;
}

class RowGrid {
    static function main() {
        var offscreen = Offscreen.create();
        for (scale in [1, 2]) {
            var tree = new LayoutTree();
            var cells = [for (_ in 0...5) new Div({}, tree)];
            var rows = [new Div({}, [cells[0], cells[1]], tree), new Div({}, [cells[2]], tree),
                new Div({}, [cells[3], cells[4]], tree)];
            var root = new Div({width: 100}, [rows[0], rows[1], rows[2]], tree);
            Identity.of(tree, root.node.id).setId("group");
            for (row in rows) Identity.of(tree, row.node.id).setClasses(["row"]);
            for (i in 0...cells.length) Identity.of(tree, cells[i].node.id).setId('c$i');
            var sheet = Css.load('#group {flex-direction:column;} .row {display:grid;grid-template-columns:40px 60px;background:#00ff00;}'
                + '#c0 {grid-column:1;grid-row:1 / span 2;min-height:80px;background:#ff0000;}'
                + '#c1, #c2, #c4 {grid-column:2;min-height:20px;} #c3 {grid-column:1;min-height:20px;}');
            var ids = haxe.io.Bytes.alloc(24);
            for (i in 0...rows.length) ids.setInt64(i * 8, rows[i].node.id);
            if (!RowGridNative.blinc_tree_set_grid_rows(tree.ptr, root.node.id, ids.getData(), 3)) throw "row registration failed";
            var duplicate = haxe.io.Bytes.alloc(16);
            duplicate.setInt64(0, rows[0].node.id); duplicate.setInt64(8, rows[0].node.id);
            if (RowGridNative.blinc_tree_set_grid_rows(tree.ptr, root.node.id, duplicate.getData(), 2)) throw "duplicate rows accepted";
            offscreen.scale = scale;
            var texture = offscreen.createTexture(100 * scale, 160 * scale);
            var view = texture.createView(new GpuTextureViewDescriptor());
            offscreen.render(root, view, 100, 160);
            var pixels = offscreen.readRgba8(texture, 100 * scale, 160 * scale);
            var offset = (60 * scale * 100 * scale + 10 * scale) * 4;
            if (pixels.get(offset) != 255 || pixels.get(offset + 1) != 0) throw "later row background covers the span";
            var hits = tree.hitTest(10, 60);
            if (hits.length != 3 || hits[0].id != cells[0].node.id || hits[1].id != rows[0].node.id) throw "span hit ancestry changed";
            if (!RowGridNative.blinc_tree_set_grid_rows(tree.ptr, root.node.id, null, 0)) throw "row reset failed";
            Sys.println('PASS: HashLink row-grid span pixels, hits, validation and reset ${scale}x');
            view.destroy(); texture.destroy(); Css.remove(sheet); root.remove(); tree.dispose();
        }
    }
}
