use std::{collections::HashMap, path::Path, sync::OnceLock};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as Base64Engine, Engine as _};
use bevy::{gltf::GltfLoaderSettings, prelude::*};
use fontdue::{
    layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle},
    Font, FontSettings,
};
use tokio::runtime::Runtime;

use crate::{
    LogPanel, ModelCache, PrimitiveMaterialCache, PrimitiveMaterialKey, TextMaterialCache,
    TextMaterialKey,
};

// ─── Transform helper (from render/mod.rs) ──────────────────────────────────

use virtual_dom::dom::element::Transform2;

pub fn apply_transform(node: &Transform2, transform: &mut Transform) {
    transform.translation = Vec3::new(node.position.x, node.position.y, node.position.z);
    transform.rotation = Quat::from_euler(
        EulerRot::XYZ,
        node.rotation.x,
        node.rotation.y,
        node.rotation.z,
    );
    transform.scale = Vec3::new(node.scale.x, node.scale.y, node.scale.z);
}

// ─── Attribute helpers ───────────────────────────────────────────────────────

pub fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    Some(Color::srgb(r, g, b))
}

pub fn get_attr_f32(attrs: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    attrs
        .get(key)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(default)
}

pub fn get_attr_string(attrs: &HashMap<String, String>, key: &str, default: &str) -> String {
    attrs
        .get(key)
        .map(|s| s.to_string())
        .unwrap_or_else(|| default.to_string())
}

// ─── Font & text texture ─────────────────────────────────────────────────────

pub fn get_text_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        let font_bytes: &[u8] = include_bytes!("../assets/fonts/FiraSans-Regular.ttf");
        Font::from_bytes(font_bytes, FontSettings::default())
            .expect("Failed to load FiraSans-Regular.ttf")
    })
}

pub fn create_text_texture(text: &str, color: Color, images: &mut Assets<Image>) -> Handle<Image> {
    let text_font = get_text_font();
    let normalized_text = if text.is_empty() { " " } else { text };
    let font_px = 48.0f32;
    let padding = 4u32;

    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: 0.0,
        y: 0.0,
        ..LayoutSettings::default()
    });
    layout.append(&[text_font], &TextStyle::new(normalized_text, font_px, 0));

    let glyphs = layout.glyphs();
    let mut max_x = 0f32;
    let mut max_y = 0f32;
    for glyph in glyphs {
        max_x = max_x.max(glyph.x + glyph.width as f32);
        max_y = max_y.max(glyph.y + glyph.height as f32);
    }

    let width = (max_x.ceil() as u32 + padding * 2).max(32);
    let height = (max_y.ceil() as u32 + padding * 2).max(16);
    let mut data = vec![0u8; (width * height * 4) as usize];

    let color_array = color.to_srgba().to_u8_array();
    let (r, g, b) = (color_array[0], color_array[1], color_array[2]);

    for glyph in glyphs {
        let (_, bitmap) = text_font.rasterize_config(glyph.key);
        let base_x = padding as i32 + glyph.x.floor() as i32;
        let base_y = padding as i32 + glyph.y.floor() as i32;

        for gy in 0..glyph.height {
            for gx in 0..glyph.width {
                let alpha = bitmap[gy * glyph.width + gx];
                if alpha == 0 {
                    continue;
                }
                let x = base_x + gx as i32;
                let y = base_y + gy as i32;
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let idx = ((y as u32 * width + x as u32) * 4) as usize;
                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = alpha;
            }
        }
    }

    images.add(Image::new(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::render::render_asset::RenderAssetUsages::RENDER_WORLD,
    ))
}

pub fn parse_text_attrs(attrs_map: &HashMap<String, String>) -> (String, f32, Color) {
    let text_value = get_attr_string(attrs_map, "value", "Text");
    let text_size = get_attr_f32(attrs_map, "size", 0.1);
    let text_color = attrs_map
        .get("color")
        .and_then(|c| parse_hex_color(c))
        .unwrap_or(Color::srgb(1.0, 1.0, 1.0));
    (text_value, text_size, text_color)
}

pub fn build_text_transform(
    mut base_transform: Transform,
    text_value: &str,
    text_size: f32,
) -> Transform {
    let text_width = text_size * text_value.chars().count() as f32 * 0.6;
    let text_height = text_size;
    base_transform.scale = Vec3::new(text_width.max(0.01), text_height.max(0.01), 1.0);
    base_transform
}

pub fn get_or_create_text_material(
    text_cache: &mut TextMaterialCache,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    text_value: &str,
    text_size: f32,
    text_color: Color,
) -> Handle<StandardMaterial> {
    let [r, g, b, a] = text_color.to_srgba().to_u8_array();
    let cache_key = TextMaterialKey {
        value: text_value.to_string(),
        size_bits: text_size.to_bits(),
        color_key: format!("{r:02x}{g:02x}{b:02x}{a:02x}"),
    };

    if let Some(handle) = text_cache.materials.get(&cache_key) {
        return handle.clone();
    }

    let text_texture = create_text_texture(text_value, text_color, images);
    let text_material = materials.add(StandardMaterial {
        base_color_texture: Some(text_texture),
        alpha_mode: bevy::prelude::AlphaMode::Blend,
        unlit: true,
        ..Default::default()
    });

    text_cache
        .materials
        .insert(cache_key, text_material.clone());
    text_material
}

pub fn get_or_create_primitive_material(
    primitive_cache: &mut PrimitiveMaterialCache,
    materials: &mut Assets<StandardMaterial>,
    color: Color,
    double_sided: bool,
) -> Handle<StandardMaterial> {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    let cache_key = PrimitiveMaterialKey {
        color_key: format!("{r:02x}{g:02x}{b:02x}{a:02x}"),
        double_sided,
    };

    if let Some(handle) = primitive_cache.materials.get(&cache_key) {
        return handle.clone();
    }

    let material = materials.add(StandardMaterial {
        base_color: color,
        cull_mode: if double_sided {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        },
        ..Default::default()
    });

    primitive_cache.materials.insert(cache_key, material.clone());
    material
}

// ─── Model loading & cache ───────────────────────────────────────────────────

pub fn encode_url_to_filename(url: &str) -> String {
    let b64 = Base64Engine.encode(url);
    let safe_b64 = b64.replace('/', "_").replace('+', "-");
    let ext = match Path::new(url).extension() {
        Some(e) => e.to_string_lossy().to_string(),
        None => "bin".to_string(),
    };
    format!("{safe_b64}.{ext}")
}

pub async fn load_bytes_from_url(url: &str) -> Result<Vec<u8>> {
    let resp = reqwest::get(url).await?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!(
            "Status {} downloading {}",
            resp.status(),
            url
        ));
    }
    Ok(resp.bytes().await?.to_vec())
}

pub fn download_model_if_needed(
    rt: &Runtime,
    url: &str,
    cache: &mut ModelCache,
    log_panel: &mut LogPanel,
) -> Result<String> {
    if let Some(cached_path) = cache.cache.get(url) {
        if Path::new(cached_path).exists() {
            log_panel.push_info(format!("Cache hit: {} -> {}", url, cached_path));
            return Ok(cached_path.clone());
        }
        log_panel.push_warn(format!(
            "Cache stale: {} -> {}. Re-downloading.",
            url, cached_path
        ));
    } else {
        log_panel.push_info(format!("Cache miss, downloading: {}", url));
    }

    let (assets_dir, cache_dir) = crate::utils::folder::resolve_assets_and_cache_dirs();
    log_panel.push_info(format!("Assets dir: {}", assets_dir.display()));
    log_panel.push_info(format!("Cache dir:  {}", cache_dir.display()));

    let _ = std::fs::create_dir_all(cache_dir.clone());
    let filename = encode_url_to_filename(url);
    let local_path = cache_dir.join(filename).to_string_lossy().to_string();

    if url.starts_with("http://") || url.starts_with("https://") {
        log_panel.push_info(format!("Downloading: {}", url));
        let bytes = rt
            .block_on(load_bytes_from_url(url))
            .map_err(|e| anyhow::anyhow!("Download failed: {e}"))?;
        std::fs::write(&local_path, bytes).map_err(|e| anyhow::anyhow!("Write failed: {e}"))?;
    } else {
        let from = std::path::PathBuf::from(url);
        if !from.exists() {
            return Err(anyhow::anyhow!("Local file not found: {url}"));
        }
        std::fs::copy(&from, &local_path).map_err(|e| anyhow::anyhow!("Copy failed: {e}"))?;
    }

    cache.cache.insert(url.to_string(), local_path.clone());
    log_panel.push_info(format!("Cached: {} -> {}", url, local_path));
    Ok(local_path)
}

pub fn apply_model_with_cache(
    remote_path: &str,
    asset_server: &AssetServer,
    log_panel: &mut LogPanel,
    model_cache: &mut ModelCache,
    rt: &Runtime,
) -> Handle<Scene> {
    match download_model_if_needed(rt, remote_path, model_cache, log_panel) {
        Ok(local_path) => {
            log_panel.push_info(format!("Model ready: {local_path}"));
            let relative: String = local_path
                .strip_prefix("crates/luna/assets/")
                .map(|s| s.to_string())
                .unwrap_or_else(|| local_path.clone());
            log_panel.push_info(format!("Loading: '{relative}'"));

            if relative.ends_with(".gltf") || relative.ends_with(".glb") {
                let final_path = format!("{relative}#Scene0");
                asset_server.load_with_settings(final_path, |s: &mut GltfLoaderSettings| {
                    s.load_cameras = false;
                    s.load_lights = false;
                })
            } else {
                asset_server.load_with_settings(relative, |s: &mut GltfLoaderSettings| {
                    s.load_cameras = false;
                    s.load_lights = false;
                })
            }
        }
        Err(e) => {
            log_panel.push_error(format!("Model prepare failed '{}': {e}", remote_path));
            Handle::default()
        }
    }
}

// ─── URL resolution ──────────────────────────────────────────────────────────

use crate::routes::VirtualRoutes;
use url::Url;

pub fn resolve_remote_path(base_url: &str, remote_path: &str) -> Option<String> {
    if VirtualRoutes::is_virtual_url(remote_path) {
        return Some(remote_path.to_string());
    }
    if remote_path.starts_with("http://") || remote_path.starts_with("https://") {
        return Some(remote_path.to_string());
    }
    let Ok(base) = Url::parse(base_url) else {
        return None;
    };
    let Ok(final_url) = base.join(remote_path) else {
        return None;
    };
    Some(final_url.to_string())
}

// ─── Tests ───────────────────────────────��───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // parse_hex_color
    #[test]
    fn hex_color_basic() {
        let c = parse_hex_color("#FF0000").unwrap();
        let s = c.to_srgba();
        assert!((s.red - 1.0).abs() < 0.01);
        assert!(s.green < 0.01);
        assert!(s.blue < 0.01);
    }

    #[test]
    fn hex_color_without_hash() {
        assert!(parse_hex_color("FF0000").is_none()); // must start with #
    }

    #[test]
    fn hex_color_black_white() {
        let black = parse_hex_color("#000000").unwrap().to_srgba();
        assert!(black.red < 0.01 && black.green < 0.01 && black.blue < 0.01);

        let white = parse_hex_color("#FFFFFF").unwrap().to_srgba();
        assert!((white.red - 1.0).abs() < 0.01);
    }

    #[test]
    fn hex_color_invalid_length() {
        assert!(parse_hex_color("#FFF").is_none());
        assert!(parse_hex_color("#FFFFFFF").is_none());
        assert!(parse_hex_color("").is_none());
    }

    #[test]
    fn hex_color_lowercase() {
        assert!(parse_hex_color("#4caf50").is_some());
    }

    // get_attr_f32
    #[test]
    fn attr_f32_present() {
        let mut m = HashMap::new();
        m.insert("size".to_string(), "3.14".to_string());
        assert!((get_attr_f32(&m, "size", 0.0) - 3.14).abs() < 0.001);
    }

    #[test]
    fn attr_f32_missing_uses_default() {
        let m = HashMap::new();
        assert_eq!(get_attr_f32(&m, "size", 1.0), 1.0);
    }

    #[test]
    fn attr_f32_invalid_uses_default() {
        let mut m = HashMap::new();
        m.insert("size".to_string(), "not_a_number".to_string());
        assert_eq!(get_attr_f32(&m, "size", 5.0), 5.0);
    }

    // get_attr_string
    #[test]
    fn attr_string_present() {
        let mut m = HashMap::new();
        m.insert("value".to_string(), "hello".to_string());
        assert_eq!(get_attr_string(&m, "value", "default"), "hello");
    }

    #[test]
    fn attr_string_missing_uses_default() {
        let m = HashMap::new();
        assert_eq!(get_attr_string(&m, "value", "default"), "default");
    }

    // encode_url_to_filename
    #[test]
    fn encode_url_no_slashes_in_name() {
        let name = encode_url_to_filename("http://example.com/model.glb");
        assert!(!name.contains('/'));
        assert!(name.ends_with(".glb"));
    }

    #[test]
    fn encode_url_no_plus_in_name() {
        let name = encode_url_to_filename("http://example.com/a+b.glb");
        assert!(!name.contains('+'));
    }

    #[test]
    fn encode_url_no_extension_uses_bin() {
        let name = encode_url_to_filename("http://example.com/resource");
        assert!(name.ends_with(".bin"));
    }

    #[test]
    fn encode_url_deterministic() {
        let url = "http://example.com/model.glb";
        assert_eq!(encode_url_to_filename(url), encode_url_to_filename(url));
    }

    // resolve_remote_path
    #[test]
    fn resolve_absolute_http_passthrough() {
        let result = resolve_remote_path("luna://home", "http://example.com/scene.hsml");
        assert_eq!(result.unwrap(), "http://example.com/scene.hsml");
    }

    #[test]
    fn resolve_absolute_https_passthrough() {
        let result = resolve_remote_path("http://base.com/page", "https://cdn.com/model.glb");
        assert_eq!(result.unwrap(), "https://cdn.com/model.glb");
    }

    #[test]
    fn resolve_virtual_url_passthrough() {
        let result = resolve_remote_path("http://base.com/", "luna://home");
        assert_eq!(result.unwrap(), "luna://home");
    }

    #[test]
    fn resolve_relative_path() {
        let result = resolve_remote_path("http://base.com/spaces/", "models/tree.glb");
        assert_eq!(result.unwrap(), "http://base.com/spaces/models/tree.glb");
    }

    #[test]
    fn resolve_invalid_base_returns_none() {
        let result = resolve_remote_path("not_a_url", "models/tree.glb");
        assert!(result.is_none());
    }
}
