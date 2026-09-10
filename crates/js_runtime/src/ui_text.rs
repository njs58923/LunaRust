//! Native text layout service supplied by the host. No DOM or cross-isolate handles.
use deno_core::{op2, OpState};
use serde::Serialize;
#[derive(Clone, Serialize)]
pub struct GlyphQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub u: f32,
    pub v: f32,
    pub uw: f32,
    pub vh: f32,
    pub page: usize,
}
#[derive(Clone, Serialize)]
pub struct TextLayout {
    pub w: f32,
    pub h: f32,
    pub glyphs: Vec<GlyphQuad>,
}
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
