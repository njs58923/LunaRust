# UI graphics

Shared font layout/atlas and bounded vector tessellation for Luna isolates.

## Vector paths

```js
const geometry = PathGeometry.tessellate([
  {op: "moveTo", x: 0, y: 0},
  {op: "bezierTo", c1x: 0, c1y: 0.3, c2x: 0.3, c2y: 0.3, x: 0.3, y: 0},
  {op: "close"}
], {tolerance: 0.0001, strokeWidth: 0, nonZero: false});
```

Commands: `moveTo`/`lineTo` (`x,y`), `quadraticTo` (`cx,cy,x,y`), `bezierTo` (`c1x,c1y,c2x,c2y,x,y`) and `close`. Start each subpath with `moveTo`.

Returns `{positions, indices, contours, closed}`. Positions are XY pairs; indices have positive XY winding. Convert positions to XYZ when building a MeshResource. Contours describe flattened input subpaths, not the outline of the generated stroke. No entities or mesh resources are allocated automatically.

Zero strokeWidth fills with EvenOdd (supports holes), or NonZero when selected. Positive strokeWidth generates round caps and joins. The synchronous operation flattens curves at the requested tolerance and tessellates with Lyon; cache geometry and call again only when the shape changes. It does not schedule frames.

Limits: 1–128 commands, finite coordinates within ±1000, tolerance 0.00001–0.1, strokeWidth 0–10, at most 512 flattened events and 65536 vertices/196608 indices. A u16 tessellation builder stops vertex growth before conversion to public u32 indices. Excessive input returns an error; split complex drawings into multiple paths.

The server_ui framework integrates these buffers into retained panel meshes, adaptive rounded backgrounds, rectangular clipping and rounded background extrusion. Path/Ellipse controls remain planar. See that project's RENDERING.md and /curves.hsml demo. Rebuild Luna to expose PathGeometry.

Validation: `cargo test -p ui_graphics -j 1`; integration compilation: `cargo check -p luna --tests -j 1`.
