//! Shared font atlas for mesh UI. Immutable glyph coordinates; uploads only after insertion.
use fontdue::layout::{CoordinateSystem, GlyphRasterConfig, Layout, LayoutSettings, TextStyle};
use fontdue::{Font, FontSettings};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
pub const SIDE: usize = 1024;
const MAX_PAGES: usize = 8;
const FONT_PX: f32 = 48.0;
const LINE: f32 = 56.0;
#[derive(Clone, Copy)]
struct Slot {
    page: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}
struct Page {
    pixels: Vec<u8>,
    x: usize,
    y: usize,
    row: usize,
    revision: u64,
}
#[derive(Default)]
struct Atlas {
    pages: Vec<Page>,
    glyphs: HashMap<GlyphRasterConfig, Slot>,
}
static ATLAS: OnceLock<Mutex<Atlas>> = OnceLock::new();
static EPOCH: AtomicU64 = AtomicU64::new(0);
fn atlas() -> &'static Mutex<Atlas> {
    ATLAS.get_or_init(Default::default)
}
impl Atlas {
    fn insert(&mut self, key: GlyphRasterConfig) -> Result<Slot, String> {
        if let Some(slot) = self.glyphs.get(&key) {
            return Ok(*slot);
        }
        let (m, bitmap) = get_text_font().rasterize_config(key);
        if m.width + 2 > SIDE || m.height + 2 > SIDE {
            return Err("UI glyph exceeds atlas page".into());
        }
        let needs_page = self.pages.last().map_or(true, |p| {
            let y = if p.x + m.width + 2 > SIDE {
                p.y + p.row
            } else {
                p.y
            };
            y + m.height + 2 > SIDE
        });
        if needs_page {
            if self.pages.len() >= MAX_PAGES {
                return Err("UI font atlas capacity reached".into());
            }
            self.pages.push(Page {
                pixels: [255,255,255,0].repeat(SIDE*SIDE),
                x: 0,
                y: 0,
                row: 0,
                revision: 0,
            });
        }
        let page = self.pages.len() - 1;
        let p = &mut self.pages[page];
        if p.x + m.width + 2 > SIDE {
            p.y += p.row;
            p.x = 0;
            p.row = 0;
        }
        let slot = Slot {
            page,
            x: p.x + 1,
            y: p.y + 1,
            w: m.width,
            h: m.height,
        };
        for y in 0..m.height {
            for x in 0..m.width {
                let i = ((slot.y + y) * SIDE + slot.x + x) * 4;
                p.pixels[i..i + 4].copy_from_slice(&[255, 255, 255, bitmap[y * m.width + x]]);
            }
        }
        p.x += m.width + 2;
        p.row = p.row.max(m.height + 2);
        p.revision += 1;
        self.glyphs.insert(key, slot);
        EPOCH.fetch_add(1, Ordering::Release);
        Ok(slot)
    }
}
pub fn layout(text: &str, size: f32, width: Option<f32>) -> Result<TextLayout, String> {
    if text.len() > 16384
        || !size.is_finite()
        || size < 0.00001
        || size > 100.0
        || width.is_some_and(|w| !w.is_finite() || w <= 0.0 || w > 10000.0)
    {
        return Err("Invalid text layout bounds".into());
    }
    let scale = size / LINE;
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        max_width: width.map(|w| w / scale),
        ..Default::default()
    });
    layout.append(&[get_text_font()], &TextStyle::new(text, FONT_PX, 0));
    let mut atlas = atlas().lock().map_err(|_| "UI atlas lock poisoned")?;
    let mut out = TextLayout {
        w: 0.0,
        h: if text.is_empty() { 0.0 } else { size },
        glyphs: Vec::new(),
    };
    for g in layout.glyphs() {
        out.w = out.w.max((g.x + g.width as f32) * scale);
        out.h = out.h.max((g.y + g.height as f32) * scale);
        if g.width == 0 || g.height == 0 {
            continue;
        }
        let s = atlas.insert(g.key)?;
        out.glyphs.push(GlyphQuad {
            x: g.x * scale,
            y: g.y * scale,
            w: g.width as f32 * scale,
            h: g.height as f32 * scale,
            u: s.x as f32 / SIDE as f32,
            v: s.y as f32 / SIDE as f32,
            uw: s.w as f32 / SIDE as f32,
            vh: s.h as f32 / SIDE as f32,
            page: s.page,
        });
    }
    Ok(out)
}
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
pub fn get_text_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(
            include_bytes!("../../luna/assets/fonts/FiraSans-Regular.ttf") as &[u8],
            FontSettings::default(),
        )
        .expect("bundled UI font")
    })
}
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}
/// Copy changed pages only; glyph UVs never move while a resource is alive.
pub fn changed_pages(revisions: &HashMap<usize, u64>) -> Vec<(usize, u64, Vec<u8>)> {
    atlas()
        .lock()
        .unwrap()
        .pages
        .iter()
        .enumerate()
        .filter(|(i, p)| revisions.get(i).copied() != Some(p.revision))
        .map(|(i, p)| (i, p.revision, p.pixels.clone()))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ui_text_real_metrics_and_stable_atlas() {
        let narrow = layout("iiii", 0.1, None).unwrap();
        let wide = layout("MMMM", 0.1, None).unwrap();
        assert!(wide.w > narrow.w * 2.0);
        let again = layout("iiii", 0.1, None).unwrap();
        assert_eq!(narrow.glyphs[0].u, again.glyphs[0].u);
        let wrapped = layout("hello world hello world", 0.1, Some(0.3)).unwrap();
        assert!(wrapped.h > 0.1);
    }
}
