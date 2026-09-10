//! Shared textures and immutable material variants. DOM instances own only bindings.
use crate::{
    models::{ModelInstance, ModelStatus},
    AttributeUpdates, ElemenetWorld, EntityMap, IoService, LogPanel, TokioRuntime, VirtualDomData,
};
use bevy::{
    asset::load_internal_asset,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::{
        mesh::MeshVertexBufferLayoutRef,
        render_asset::RenderAssetUsages,
        render_resource::{
            AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderRef, ShaderType,
            SpecializedMeshPipelineError, TextureDimension, TextureFormat,
        },
        texture::ImageSampler,
    },
};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

const SHADER: Handle<Shader> = Handle::weak_from_u128(0x4262feaeec444dd5ac76e317e3fc0042);
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 4096 * 4096;
const MAX_TEXTURES: usize = 128;
const MAX_MATERIALS: usize = 1024;
const MAX_TEXTURE_MEMORY: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Fit {
    #[default]
    Stretch,
    Contain,
    Cover,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceDesc {
    pub source: Option<String>,
    pub raw_source: String,
    pub revision: String,
    pub region: [u32; 4],
    pub fit: Fit,
    pub padding: u32,
    pub front: bool,
    pub overlay: bool,
    pub unlit: Option<bool>,
    pub alpha: Option<String>,
    pub tint: Option<[u8; 4]>,
    pub image: bool,
    pub error: Option<String>,
}
impl SurfaceDesc {
    pub fn parse(
        tag: &str,
        attrs: &HashMap<String, String>,
        resolve: impl FnOnce(&str) -> Option<String>,
    ) -> Option<Self> {
        let image = tag == "image";
        let source = attrs
            .get(if image { "src" } else { "texture" })
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if !image
            && source.is_none()
            && !attrs.contains_key("material-unlit")
            && !attrs.contains_key("material-alpha")
            && !(tag == "model" && attrs.contains_key("color"))
        {
            return None;
        }
        let mut error = None;
        let region = match attrs.get("texture-region") {
            None => [0.0, 0.0, 1.0, 1.0],
            Some(raw) => {
                let v: Vec<f32> = raw
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or_default();
                if v.len() != 4
                    || v.iter().any(|n| !n.is_finite())
                    || v[0] < 0.0
                    || v[1] < 0.0
                    || v[2] <= 0.0
                    || v[3] <= 0.0
                    || v[0] + v[2] > 1.00001
                    || v[1] + v[3] > 1.00001
                {
                    error = Some(
                        "texture-region requires normalized x,y,width,height inside [0,1]".into(),
                    );
                    [0.0, 0.0, 1.0, 1.0]
                } else {
                    [v[0], v[1], v[2], v[3]]
                }
            }
        };
        let fit = match attrs
            .get(if image { "fit" } else { "texture-fit" })
            .map(String::as_str)
            .unwrap_or(if image { "contain" } else { "stretch" })
        {
            "contain" => Fit::Contain,
            "cover" => Fit::Cover,
            "stretch" => Fit::Stretch,
            _ => {
                error = Some("fit must be contain, cover or stretch".into());
                Fit::Stretch
            }
        };
        let padding = attrs
            .get("texture-padding")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0);
        if !padding.is_finite() || !(0.0..0.5).contains(&padding) {
            error = Some("texture-padding must be in [0,0.5)".into());
        }
        let resolved = source.and_then(resolve);
        if source.is_some() && resolved.is_none() {
            error = Some("Cannot resolve texture URL".into());
        }
        let alpha = attrs.get("material-alpha").cloned();
        if attrs
            .get("texture-face")
            .is_some_and(|s| !matches!(s.as_str(), "front" | "all"))
        {
            error = Some("texture-face must be front or all".into());
        }
        if attrs
            .get("material-unlit")
            .is_some_and(|s| !matches!(s.as_str(), "true" | "false"))
        {
            error = Some("material-unlit must be true or false".into());
        }
        if alpha
            .as_deref()
            .is_some_and(|s| !matches!(s, "opaque" | "mask" | "blend"))
        {
            error = Some("material-alpha must be opaque, mask or blend".into());
        }
        let tint = attrs
            .get("color")
            .and_then(|s| crate::render::parse_hex_color(s))
            .map(|c| c.to_srgba().to_u8_array());
        Some(Self {
            source: resolved,
            raw_source: source.unwrap_or("").into(),
            revision: attrs.get("texture-revision").cloned().unwrap_or_default(),
            region: region.map(f32::to_bits),
            fit,
            padding: padding.clamp(0.0, 0.49).to_bits(),
            front: attrs.get("texture-face").is_some_and(|s| s == "front"),
            overlay: !image && attrs.get("texture-face").is_some_and(|s| s == "front"),
            unlit: attrs
                .get("material-unlit")
                .map(|s| s == "true")
                .or(image.then_some(true)),
            alpha,
            tint,
            image,
            error,
        })
    }
}

#[derive(Resource, Default)]
pub struct SurfaceQueue(pub Vec<(specs::Entity, Option<SurfaceDesc>)>);
#[derive(Component, Clone, PartialEq)]
struct SurfaceRequest {
    node: specs::Entity,
    desc: Option<SurfaceDesc>,
    model_generation: u64,
}
#[derive(Component)]
struct OriginalMaterial(Handle<StandardMaterial>);

#[derive(Clone, Copy, ShaderType)]
pub struct SurfaceUniform {
    region: Vec4,
    // x: fit (0 stretch,1 contain,2 cover), y: texture aspect, z: padding, w: front only
    mapping: Vec4,
    // x: texture enabled, y: overlay vs multiply, zw reserved
    options: Vec4,
}
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct SurfaceExtension {
    #[uniform(100)]
    pub settings: SurfaceUniform,
    #[texture(101)]
    #[sampler(102)]
    pub texture: Option<Handle<Image>>,
}
impl MaterialExtension for SurfaceExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor
            .vertex
            .shader_defs
            .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
        }
        Ok(())
    }
}
pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceExtension>;

#[derive(Clone)]
enum TextureState {
    Loading,
    Ready {
        image: Handle<Image>,
        width: u32,
        height: u32,
    },
    Error(String),
}
struct TextureEntry {
    state: TextureState,
    touched: Instant,
}
struct Decoded {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}
struct TextureResult {
    key: String,
    network: u64,
    data: Result<Decoded, String>,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct MaterialKey {
    original: AssetId<StandardMaterial>,
    desc: SurfaceDesc,
    texture: Option<AssetId<Image>>,
}
#[derive(Resource)]
pub struct SurfaceCache {
    textures: HashMap<String, TextureEntry>,
    materials: HashMap<MaterialKey, (Handle<SurfaceMaterial>, Instant)>,
    pending: HashSet<Entity>,
    allocations: HashMap<AssetId<Image>, usize>,
    permits: Arc<tokio::sync::Semaphore>,
    last_sweep: Instant,
    tx: mpsc::SyncSender<TextureResult>,
    rx: Mutex<mpsc::Receiver<TextureResult>>,
}
impl Default for SurfaceCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(MAX_TEXTURES);
        Self {
            textures: HashMap::new(),
            materials: HashMap::new(),
            pending: HashSet::new(),
            allocations: HashMap::new(),
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
            last_sweep: Instant::now(),
            tx,
            rx: Mutex::new(rx),
        }
    }
}
pub struct SurfacePlugin;
impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurfaceQueue>()
            .init_resource::<SurfaceCache>()
            .add_plugins(MaterialPlugin::<SurfaceMaterial> {
                prepass_enabled: false,
                shadows_enabled: false,
                ..default()
            });
        load_internal_asset!(app, SHADER, "surface.wgsl", Shader::from_wgsl);
    }
}

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Image exceeds 16 MiB encoded limit".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(MAX_PIXELS * 8);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let (width, height) = (image.width(), image.height());
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("Image exceeds pixel budget".into());
    }
    Ok(Decoded {
        width,
        height,
        rgba: image.into_rgba8().into_raw(),
    })
}

fn texture_key(desc: &SurfaceDesc) -> String {
    format!(
        "{}\n{}",
        desc.source.as_deref().unwrap_or(""),
        desc.revision
    )
}

fn request_texture(
    world: &mut World,
    cache: &mut SurfaceCache,
    desc: &SurfaceDesc,
) -> TextureState {
    if let Some(page) = desc.source.as_deref().and_then(|s|s.strip_prefix("luna://ui-font/")).and_then(|s|s.parse::<usize>().ok()) {
        return match crate::ui_text::image(world,page) {
            Some(image)=>TextureState::Ready{image,width:1024,height:1024},
            None=>TextureState::Error("Unknown UI font atlas page".into()),
        };
    }
    let key = texture_key(desc);
    if let Some(entry) = cache.textures.get_mut(&key) {
        entry.touched = Instant::now();
        return entry.state.clone();
    }
    // Bounded resident cache; active surfaces retain their own image handles.
    if cache.textures.len() >= MAX_TEXTURES {
        let oldest = cache
            .textures
            .iter()
            .filter(|(_, e)| !matches!(e.state, TextureState::Loading))
            .min_by_key(|(_, e)| e.touched)
            .map(|(k, _)| k.clone());
        if let Some(key) = oldest {
            cache.textures.remove(&key);
        } else {
            return TextureState::Error("Too many pending textures".into());
        }
    }
    let url = desc.source.clone().unwrap_or_default();
    let io = world.resource::<IoService>();
    let network = io.begin_request(crate::io::NetworkRequestKind::Image, &url, "surface");
    let client = io.http_client();
    let tx = cache.tx.clone();
    let permits = cache.permits.clone();
    cache.textures.insert(
        key.clone(),
        TextureEntry {
            state: TextureState::Loading,
            touched: Instant::now(),
        },
    );
    world.resource::<TokioRuntime>().0.spawn(async move {
        let data = async {
            let _permit = permits.acquire().await.map_err(|e| e.to_string())?;
            let bytes = if url == "luna://icons/menu.png" {
                include_bytes!("../assets/icons/menu.png").to_vec()
            } else {
                let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err("Images accept http(s) and bundled luna assets only".into());
                }
                let mut response = client
                    .get(parsed)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .error_for_status()
                    .map_err(|e| e.to_string())?;
                if response
                    .content_length()
                    .is_some_and(|n| n > MAX_BYTES as u64)
                {
                    return Err("Image exceeds 16 MiB encoded limit".into());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                    if bytes.len() + chunk.len() > MAX_BYTES {
                        return Err("Image exceeds 16 MiB encoded limit".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                bytes
            };
            tokio::task::spawn_blocking(move || decode(&bytes))
                .await
                .map_err(|e| e.to_string())?
        }
        .await;
        let _ = tx.try_send(TextureResult { key, network, data });
    });
    TextureState::Loading
}

fn publish(
    world: &mut World,
    request: &SurfaceRequest,
    status: &str,
    error: &str,
    size: (u32, u32),
) {
    let Some(desc) = &request.desc else {
        return;
    };
    let prefix = if desc.image { "image" } else { "texture" };
    let mut changed = Vec::new();
    for (key, value) in [
        (format!("{prefix}-status"), status.into()),
        (format!("{prefix}-error"), error.into()),
        (format!("{prefix}-request-source"), desc.raw_source.clone()),
        (format!("{prefix}-request-revision"), desc.revision.clone()),
        (format!("{prefix}-width"), size.0.to_string()),
        (format!("{prefix}-height"), size.1.to_string()),
    ] {
        use specs::WorldExt;
        let dom = world.resource::<ElemenetWorld>();
        let attrs = dom.0.read_storage::<virtual_dom::dom::element::Attrs>();
        if attrs.get(request.node).and_then(|a| a.0.get(&key)) != Some(&value) {
            changed.push((request.node.id(), key, value));
        }
    }
    world.resource_mut::<AttributeUpdates>().0.extend(changed);
}

fn material(
    world: &mut World,
    cache: &mut SurfaceCache,
    original: &Handle<StandardMaterial>,
    desc: &SurfaceDesc,
    texture: Option<(Handle<Image>, u32, u32)>,
) -> Handle<SurfaceMaterial> {
    let key = MaterialKey {
        original: original.id(),
        desc: desc.clone(),
        texture: texture.as_ref().map(|(h, _, _)| h.id()),
    };
    if let Some((handle, touched)) = cache.materials.get_mut(&key) {
        *touched = Instant::now();
        return handle.clone();
    }
    let mut base = world
        .resource::<Assets<StandardMaterial>>()
        .get(original)
        .cloned()
        .unwrap_or_default();
    if let Some(tint) = desc.tint {
        base.base_color = Color::srgba_u8(tint[0], tint[1], tint[2], tint[3]);
    } else if desc.image {
        base.base_color = Color::WHITE;
    }
    if let Some(unlit) = desc.unlit {
        base.unlit = unlit;
    }
    if desc.image {
        base.cull_mode = None;
    }
    base.alpha_mode = match desc.alpha.as_deref() {
        Some("opaque") => AlphaMode::Opaque,
        Some("mask") => AlphaMode::Mask(0.5),
        Some("blend") => AlphaMode::Blend,
        _ if desc.image => AlphaMode::Blend,
        _ => base.alpha_mode,
    };
    if texture.is_some() {
        base.base_color_texture = None;
    }
    let region = Vec4::from_array(desc.region.map(f32::from_bits));
    let aspect = texture.as_ref().map_or(1.0, |(_, w, h)| {
        *w as f32 * region.z / (*h as f32 * region.w)
    });
    let extended = SurfaceMaterial {
        base,
        extension: SurfaceExtension {
            settings: SurfaceUniform {
                region,
                mapping: Vec4::new(
                    match desc.fit {
                        Fit::Stretch => 0.0,
                        Fit::Contain => 1.0,
                        Fit::Cover => 2.0,
                    },
                    aspect,
                    f32::from_bits(desc.padding),
                    if desc.front { 1.0 } else { 0.0 },
                ),
                options: Vec4::new(
                    if texture.is_some() { 1.0 } else { 0.0 },
                    if desc.overlay { 1.0 } else { 0.0 },
                    0.0,
                    0.0,
                ),
            },
            texture: texture.map(|(h, _, _)| h),
        },
    };
    let handle = world
        .resource_mut::<Assets<SurfaceMaterial>>()
        .add(extended);
    if cache.materials.len() >= MAX_MATERIALS {
        if let Some(key) = cache
            .materials
            .iter()
            .min_by_key(|(_, (_, t))| *t)
            .map(|(k, _)| k.clone())
        {
            cache.materials.remove(&key);
        }
    }
    cache
        .materials
        .insert(key, (handle.clone(), Instant::now()));
    handle
}

fn targets(world: &World, root: Entity) -> Option<Vec<Entity>> {
    let Some(instance) = world.get::<ModelInstance>(root) else {
        return Some(vec![root]);
    };
    if instance.status == ModelStatus::Loading {
        return None;
    }
    let mut stack: Vec<_> = instance.content.into_iter().collect();
    let mut result = Vec::new();
    while let Some(entity) = stack.pop() {
        if world.get::<Handle<Mesh>>(entity).is_some() {
            result.push(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().copied());
        }
    }
    Some(result)
}

fn hide_image(world: &mut World, entity: Entity) {
    if let Some(original) = world.get::<Handle<StandardMaterial>>(entity).cloned() {
        world.entity_mut(entity).insert(OriginalMaterial(original));
    }
    world
        .entity_mut(entity)
        .remove::<Handle<StandardMaterial>>()
        .remove::<Handle<SurfaceMaterial>>();
}

/// After dom_sync / scene readiness. Transform-only work never enters this queue.
pub fn sync_surfaces(world: &mut World) {
    crate::ui_text::sync(world);
    if !world.contains_resource::<SurfaceCache>() {
        return;
    }
    if world.resource::<SurfaceQueue>().0.is_empty()
        && world.resource::<SurfaceCache>().pending.is_empty()
        && world.resource::<SurfaceCache>().last_sweep.elapsed() < Duration::from_secs(1)
    {
        return;
    }
    let queued = std::mem::take(&mut world.resource_mut::<SurfaceQueue>().0);
    for (node, desc) in queued {
        if world.resource::<VirtualDomData>().nodes.get(&node.id()) != Some(&node) {
            continue;
        }
        if let Some(&entity) = world.resource::<EntityMap>().0.get(&node.id()) {
            if desc.is_none() && world.get::<SurfaceRequest>(entity).is_none() {
                continue;
            }
            let model_generation = world
                .get::<ModelInstance>(entity)
                .map_or(0, |m| m.generation);
            let request = SurfaceRequest {
                node,
                desc,
                model_generation,
            };
            if world.get::<SurfaceRequest>(entity) != Some(&request) {
                world.entity_mut(entity).insert(request);
                world.resource_mut::<SurfaceCache>().pending.insert(entity);
            } else if world
                .get::<SurfaceRequest>(entity)
                .and_then(|r| r.desc.as_ref())
                .is_some_and(|d| d.image)
                && world.get::<Handle<SurfaceMaterial>>(entity).is_none()
            {
                hide_image(world, entity);
            } else if world.get::<Handle<StandardMaterial>>(entity).is_some()
                && world.get::<Handle<SurfaceMaterial>>(entity).is_some()
            {
                // DOM color/shape refresh re-attached the standard material.
                world
                    .entity_mut(entity)
                    .remove::<Handle<StandardMaterial>>();
            }
        }
    }
    world.resource_scope(|world, mut cache: Mut<SurfaceCache>| {
        let results: Vec<_> = cache.rx.lock().unwrap().try_iter().collect();
        for result in results {
            cache
                .allocations
                .retain(|id, _| world.resource::<Assets<Image>>().contains(*id));
            let resident: usize = cache.allocations.values().sum();
            let state = match result.data {
                Ok(data) if resident + data.rgba.len() <= MAX_TEXTURE_MEMORY => {
                    let bytes = data.rgba.len();
                    let mut image = Image::new(
                        Extent3d {
                            width: data.width,
                            height: data.height,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        data.rgba,
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::default(),
                    );
                    image.sampler = ImageSampler::linear();
                    let handle = world.resource_mut::<Assets<Image>>().add(image);
                    cache.allocations.insert(handle.id(), bytes);
                    TextureState::Ready {
                        image: handle,
                        width: data.width,
                        height: data.height,
                    }
                }
                Ok(_) => {
                    TextureState::Error("Surface texture memory budget reached (256 MiB)".into())
                }
                Err(error) => TextureState::Error(error),
            };
            let (status, detail) = match &state {
                TextureState::Error(e) => (crate::io::NetworkRequestStatus::Error, Some(e.clone())),
                _ => (crate::io::NetworkRequestStatus::Ok, None),
            };
            world
                .resource::<IoService>()
                .finish_request(result.network, status, detail);
            if let Some(entry) = cache.textures.get_mut(&result.key) {
                entry.state = state;
            }
        }
        let pending: Vec<_> = cache.pending.iter().copied().collect();
        for entity in pending {
            let Some(request) = world.get::<SurfaceRequest>(entity).cloned() else {
                cache.pending.remove(&entity);
                continue;
            };
            if world
                .resource::<VirtualDomData>()
                .nodes
                .get(&request.node.id())
                != Some(&request.node)
            {
                cache.pending.remove(&entity);
                continue;
            }
            let texture = if let Some(desc) = &request.desc {
                if let Some(error) = &desc.error {
                    if desc.image {
                        hide_image(world, entity);
                    }
                    publish(world, &request, "error", error, (0, 0));
                    cache.pending.remove(&entity);
                    continue;
                }
                if desc.source.is_some() {
                    match request_texture(world, &mut cache, desc) {
                        TextureState::Loading => {
                            if desc.image {
                                hide_image(world, entity);
                            }
                            publish(world, &request, "loading", "", (0, 0));
                            continue;
                        }
                        TextureState::Error(error) => {
                            if desc.image {
                                hide_image(world, entity);
                            }
                            publish(world, &request, "error", &error, (0, 0));
                            if let Some(mut log) = world.get_resource_mut::<LogPanel>() {
                                log.push_warn(format!("Image {}: {error}", desc.raw_source));
                            }
                            cache.pending.remove(&entity);
                            continue;
                        }
                        TextureState::Ready {
                            image,
                            width,
                            height,
                        } => Some((image, width, height)),
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if request
                .desc
                .as_ref()
                .is_some_and(|d| d.image && d.source.is_none())
            {
                hide_image(world, entity);
                publish(world, &request, "empty", "", (0, 0));
                cache.pending.remove(&entity);
                continue;
            }
            let Some(targets) = targets(world, entity) else {
                continue;
            };
            for target in targets {
                let original = world
                    .get::<Handle<StandardMaterial>>(target)
                    .cloned()
                    .or_else(|| world.get::<OriginalMaterial>(target).map(|o| o.0.clone()));
                let Some(original) = original else {
                    continue;
                };
                if let Some(desc) = &request.desc {
                    let handle = material(world, &mut cache, &original, desc, texture.clone());
                    world
                        .entity_mut(target)
                        .insert((OriginalMaterial(original), handle))
                        .remove::<Handle<StandardMaterial>>();
                } else {
                    let restore = world
                        .get::<Handle<StandardMaterial>>(target)
                        .cloned()
                        .unwrap_or(original);
                    world
                        .entity_mut(target)
                        .insert(restore)
                        .remove::<Handle<SurfaceMaterial>>()
                        .remove::<OriginalMaterial>();
                }
            }
            if let Some(desc) = &request.desc {
                publish(
                    world,
                    &request,
                    if desc.image && desc.source.is_none() {
                        "empty"
                    } else {
                        "ready"
                    },
                    "",
                    texture.as_ref().map_or((0, 0), |(_, w, h)| (*w, *h)),
                );
            }
            cache.pending.remove(&entity);
        }
        // Drop cache-only assets after an idle period; mounted instances retain handles.
        if cache.last_sweep.elapsed() >= Duration::from_secs(1) {
            cache.last_sweep = Instant::now();
            cache.textures.retain(|_, e| {
                matches!(e.state, TextureState::Loading)
                    || e.touched.elapsed() < Duration::from_secs(300)
            });
            cache
                .materials
                .retain(|_, (_, t)| t.elapsed() < Duration::from_secs(300));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::{Builder, WorldExt};
    use virtual_dom::dom::element::Attrs;

    fn desc(tag: &str, values: &[(&str, &str)]) -> SurfaceDesc {
        SurfaceDesc::parse(
            tag,
            &values
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            |s| Some(format!("https://example.test/{s}")),
        )
        .unwrap()
    }
    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<SurfaceMaterial>>();
        world.init_resource::<SurfaceQueue>();
        world.init_resource::<SurfaceCache>();
        world.init_resource::<AttributeUpdates>();
        world.init_resource::<EntityMap>();
        world.init_resource::<VirtualDomData>();
        world.init_resource::<IoService>();
        world.insert_resource(ElemenetWorld(virtual_dom::dom::element::build_world()));
        world
    }
    fn instance(
        world: &mut World,
        original: Handle<StandardMaterial>,
        mesh: Handle<Mesh>,
    ) -> (specs::Entity, Entity) {
        let node = world
            .resource_mut::<ElemenetWorld>()
            .0
            .create_entity()
            .with(Attrs(HashMap::new()))
            .build();
        let entity = world.spawn((SpatialBundle::default(), original, mesh)).id();
        world
            .resource_mut::<VirtualDomData>()
            .nodes
            .insert(node.id(), node);
        world
            .resource_mut::<EntityMap>()
            .0
            .insert(node.id(), entity);
        (node, entity)
    }
    fn ready_texture(world: &mut World, desc: &SurfaceDesc) -> Handle<Image> {
        let image = world.resource_mut::<Assets<Image>>().add(Image::default());
        world.resource_mut::<SurfaceCache>().textures.insert(
            texture_key(desc),
            TextureEntry {
                state: TextureState::Ready {
                    image: image.clone(),
                    width: 256,
                    height: 128,
                },
                touched: Instant::now(),
            },
        );
        image
    }
    #[test]
    fn surface_descriptor_validates_regions_and_has_image_defaults() {
        let image = desc("image", &[("src", "icon.png")]);
        assert_eq!(image.fit, Fit::Contain);
        assert_eq!(image.unlit, Some(true));
        assert_eq!(
            image.source.as_deref(),
            Some("https://example.test/icon.png")
        );
        for value in ["0,garbage,0,1,1", "0,0,-1,1", "0.9,0,0.5,1", "NaN,0,1,1"] {
            assert!(desc("image", &[("src", "x"), ("texture-region", value)])
                .error
                .is_some());
        }
        assert!(SurfaceDesc::parse("box", &HashMap::new(), |_| None).is_none());
        assert!(desc("model", &[("material-unlit", "true")])
            .source
            .is_none());
    }
    #[test]
    fn surface_variants_share_assets_without_mutating_other_instances() {
        let mut world = world();
        let original = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(crate::utils::shapes::create_cube());
        let (a, ae) = instance(&mut world, original.clone(), mesh.clone());
        let (b, be) = instance(&mut world, original.clone(), mesh.clone());
        let shared = desc(
            "box",
            &[("texture", "atlas.png"), ("texture-face", "front")],
        );
        let texture = ready_texture(&mut world, &shared);
        world
            .resource_mut::<SurfaceQueue>()
            .0
            .extend([(a, Some(shared.clone())), (b, Some(shared.clone()))]);
        sync_surfaces(&mut world);
        let first = world.get::<Handle<SurfaceMaterial>>(ae).unwrap().clone();
        assert_eq!(Some(&first), world.get::<Handle<SurfaceMaterial>>(be));
        let changed = desc(
            "box",
            &[
                ("texture", "atlas.png"),
                ("texture-face", "front"),
                ("texture-region", "0.5,0,0.5,1"),
            ],
        );
        world
            .resource_mut::<SurfaceQueue>()
            .0
            .push((a, Some(changed)));
        sync_surfaces(&mut world);
        assert_ne!(world.get::<Handle<SurfaceMaterial>>(ae), Some(&first));
        assert_eq!(world.get::<Handle<SurfaceMaterial>>(be), Some(&first));
        assert_eq!(world.get::<Handle<Mesh>>(ae), Some(&mesh));
        assert_eq!(
            world
                .resource::<Assets<SurfaceMaterial>>()
                .get(&first)
                .unwrap()
                .extension
                .texture
                .as_ref(),
            Some(&texture)
        );
        assert!(world
            .resource::<Assets<StandardMaterial>>()
            .get(&original)
            .unwrap()
            .base_color_texture
            .is_none());
        world.resource_mut::<SurfaceQueue>().0.push((a, None));
        sync_surfaces(&mut world);
        assert_eq!(world.get::<Handle<StandardMaterial>>(ae), Some(&original));
        assert!(world.get::<Handle<SurfaceMaterial>>(ae).is_none());
    }
    #[test]
    fn surface_late_texture_result_cannot_replace_new_source_or_add_image_children() {
        let mut world = world();
        let original = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(crate::utils::shapes::create_plane());
        let (node, entity) = instance(&mut world, original, mesh.clone());
        let a = desc("image", &[("src", "slow.png")]);
        world.resource_mut::<SurfaceCache>().textures.insert(
            texture_key(&a),
            TextureEntry {
                state: TextureState::Loading,
                touched: Instant::now(),
            },
        );
        world
            .resource_mut::<SurfaceQueue>()
            .0
            .push((node, Some(a.clone())));
        sync_surfaces(&mut world);
        assert!(world.get::<Handle<StandardMaterial>>(entity).is_none());
        let b = desc("image", &[("src", "fast.png")]);
        let newest = ready_texture(&mut world, &b);
        world.resource_mut::<SurfaceQueue>().0.push((node, Some(b)));
        sync_surfaces(&mut world);
        let handle = world
            .get::<Handle<SurfaceMaterial>>(entity)
            .unwrap()
            .clone();
        world
            .resource::<SurfaceCache>()
            .tx
            .try_send(TextureResult {
                key: texture_key(&a),
                network: 0,
                data: Ok(Decoded {
                    rgba: vec![255; 4],
                    width: 1,
                    height: 1,
                }),
            })
            .unwrap();
        world.resource_mut::<SurfaceCache>().last_sweep = Instant::now() - Duration::from_secs(2);
        sync_surfaces(&mut world);
        assert_eq!(world.get::<Handle<SurfaceMaterial>>(entity), Some(&handle));
        assert_eq!(
            world
                .resource::<Assets<SurfaceMaterial>>()
                .get(&handle)
                .unwrap()
                .extension
                .texture
                .as_ref(),
            Some(&newest)
        );
        assert_eq!(world.get::<Handle<Mesh>>(entity), Some(&mesh));
        assert!(world.get::<Children>(entity).is_none());
        world
            .resource_mut::<SurfaceQueue>()
            .0
            .push((node, Some(desc("image", &[]))));
        sync_surfaces(&mut world);
        assert!(world.get::<Handle<SurfaceMaterial>>(entity).is_none());
    }
    #[test]
    fn surface_decoder_accepts_transparency_and_rejects_invalid_or_oversized_data() {
        let icon = decode(include_bytes!("../assets/icons/menu.png")).unwrap();
        assert_eq!((icon.width, icon.height), (768, 384));
        assert!(icon.rgba.chunks_exact(4).any(|p| p[3] == 0));
        assert!(icon.rgba.chunks_exact(4).any(|p| p[3] == 255));
        assert!(decode(b"not an image").is_err());
        assert!(decode(&vec![0; MAX_BYTES + 1]).is_err());
    }
}
