//! Native text layout service supplied by the host. No DOM or cross-isolate handles.
use deno_core::{op2, OpState};
pub use ui_graphics::{GlyphQuad, TextLayout};
pub struct TextBackend(pub fn(&str, f32, Option<f32>) -> Result<TextLayout, String>);
#[op2]
#[serde]
pub fn op_ui_text(
    state: &mut OpState,
    #[string] text: String,
    size: f32,
    width: f32,
) -> Result<TextLayout, anyhow::Error> {
    if text.len() > 16384
        || !size.is_finite()
        || size <= 0.0
        || size > 100.0
        || !width.is_finite()
        || width < 0.0
    {
        return Err(anyhow::anyhow!(
            "Invalid text layout: 16 KiB text, positive finite size, nonnegative width required"
        ));
    }
    let backend = state
        .try_borrow::<TextBackend>()
        .ok_or_else(|| anyhow::anyhow!("Text layout unavailable in this host"))?;
    (backend.0)(&text, size, if width > 0.0 { Some(width) } else { None })
        .map_err(anyhow::Error::msg)
}

#[op2]
#[serde]
pub fn op_ui_path(
    #[serde] request: ui_graphics::paths::Request,
) -> Result<ui_graphics::paths::Geometry, anyhow::Error> {
    ui_graphics::paths::tessellate(request).map_err(anyhow::Error::msg)
}
