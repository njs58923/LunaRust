//! Shared font atlas for mesh UI. Immutable glyph coordinates; uploads only after insertion.
use bevy::prelude::*;
use fontdue::{
    layout::{CoordinateSystem, GlyphRasterConfig, Layout, LayoutSettings, TextStyle},
};
use js_runtime::ui_text::{GlyphQuad, TextLayout};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
const SIDE: usize = 1024;
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
        let (m, bitmap) = crate::render::get_text_font().rasterize_config(key);
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
                pixels: vec![0; SIDE * SIDE * 4],
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
    let scale = size / LINE;
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        max_width: width.map(|w| w / scale),
        ..Default::default()
    });
    layout.append(
        &[crate::render::get_text_font()],
        &TextStyle::new(text, FONT_PX, 0),
    );
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
#[derive(Resource, Default)]
pub struct AtlasImages {
    pages: HashMap<usize, (u64, Handle<Image>)>,
    epoch: u64,
}
pub fn sync(world: &mut World) {
    let epoch = EPOCH.load(Ordering::Acquire);
    world.init_resource::<AtlasImages>();
    if world.resource::<AtlasImages>().epoch == epoch {
        return;
    }
    let atlas = atlas().lock().unwrap();
    world.resource_scope(|world, mut cache: Mut<AtlasImages>| {
        for (page, p) in atlas.pages.iter().enumerate() {
            if cache
                .pages
                .get(&page)
                .is_some_and(|(r, _)| *r == p.revision)
            {
                continue;
            }
            let image = Image::new(
                bevy::render::render_resource::Extent3d {
                    width: SIDE as u32,
                    height: SIDE as u32,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                p.pixels.clone(),
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::render::render_asset::RenderAssetUsages::default(),
            );
            let handle = if let Some((_, h)) = cache.pages.get(&page) {
                world.resource_mut::<Assets<Image>>().insert(h.id(), image);
                h.clone()
            } else {
                world.resource_mut::<Assets<Image>>().add(image)
            };
            cache.pages.insert(page, (p.revision, handle));
        }
        cache.epoch = epoch;
    });
}
pub fn image(world: &mut World, page: usize) -> Option<Handle<Image>> {
    sync(world);
    world
        .resource::<AtlasImages>()
        .pages
        .get(&page)
        .map(|(_, h)| h.clone())
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
