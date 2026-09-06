//! On-demand spectator rendering to a texture. Never reads or moves an XR eye.
use crate::agent::{AgentControl, CameraChoice, Reply, SpectatorCamera};
use base64::Engine;
use bevy::{
    prelude::*,
    render::{
        camera::RenderTarget,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::{RenderAssetUsages, RenderAssets},
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
        view::screenshot::ScreenshotManager,
        Render, RenderApp, RenderSet,
    },
};
use std::sync::{Arc, Mutex};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
#[derive(Resource)]
struct Target(Handle<Image>);
#[derive(Resource, Default, Clone, ExtractResource)]
struct Readback(Arc<Mutex<Option<(Handle<Image>, Reply)>>>);

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new_fill(
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);
    commands.spawn((
        Camera3dBundle {
            camera: Camera {
                target: RenderTarget::Image(handle.clone()),
                is_active: false,
                order: -1,
                ..default()
            },
            transform: Transform::from_xyz(0., 3., 8.).looking_at(Vec3::new(0., 1., 0.), Vec3::Y),
            ..default()
        },
        SpectatorCamera,
    ));
    commands.insert_resource(Target(handle));
}

fn encode(image: Image, reply: Reply) {
    bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        if reply.is_closed() { return; }
        let result = image.try_into_dynamic().map_err(|e|format!("image conversion: {e}"))
            .and_then(|image| {
                let mut png = std::io::Cursor::new(Vec::new());
                image.to_rgb8().write_to(&mut png, image::ImageFormat::Png).map_err(|e|e.to_string())?;
                Ok(serde_json::json!({"mimeType":"image/png", "data":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}))
            });
        let _ = reply.send(result);
    }).detach();
}

pub fn capture(world: &mut World, choice: CameraChoice, reply: Reply) {
    if choice == CameraChoice::Desktop {
        let active = world
            .query_filtered::<&Camera, With<crate::DesktopCamera>>()
            .iter(world)
            .any(|c| c.is_active);
        if !active {
            let _ = reply.send(Err("desktop camera inactive; use spectator in VR".into()));
            return;
        }
        let window = world
            .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
            .get_single(world);
        let Ok(window) = window else {
            let _ = reply.send(Err("no desktop window".into()));
            return;
        };
        // Share the sender so an immediate ScreenshotManager rejection is reported too.
        let slot = Arc::new(Mutex::new(Some(reply)));
        let callback_slot = slot.clone();
        let result =
            world
                .resource_mut::<ScreenshotManager>()
                .take_screenshot(window, move |image| {
                    if let Some(reply) = callback_slot.lock().unwrap().take() {
                        encode(image, reply);
                    }
                });
        if result.is_err() {
            if let Some(reply) = slot.lock().unwrap().take() {
                let _ = reply.send(Err("a desktop capture is already pending".into()));
            }
        }
        return;
    }
    let handle = world.resource::<Target>().0.clone();
    {
        let readback = world.resource::<Readback>();
        let mut pending = readback.0.lock().unwrap();
        if pending.is_some() {
            let _ = reply.send(Err("a spectator capture is already pending".into()));
            return;
        }
        *pending = Some((handle, reply));
    }
    for mut camera in world
        .query_filtered::<&mut Camera, With<SpectatorCamera>>()
        .iter_mut(world)
    {
        camera.is_active = true;
    }
}

fn idle_camera(
    control: Res<AgentControl>,
    readback: Res<Readback>,
    mut cameras: Query<&mut Camera, With<SpectatorCamera>>,
) {
    let mut pending = readback.0.lock().unwrap();
    if !control.enabled || pending.as_ref().is_some_and(|(_, r)| r.is_closed()) {
        pending.take();
    }
    for mut camera in &mut cameras {
        camera.is_active = pending.is_some();
    }
}

fn readback_frame(
    readback: Res<Readback>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    // Poll without waiting on the GPU: a VR frame must never block for automation.
    device.poll(Maintain::Poll);
    let mut pending = readback.0.lock().unwrap();
    let Some((handle, reply)) = pending.as_ref() else {
        return;
    };
    if reply.is_closed() {
        pending.take();
        return;
    }
    let Some(gpu) = images.get(handle) else {
        return;
    };
    let (_, reply) = pending.take().unwrap();
    drop(pending);
    let row = RenderDevice::align_copy_bytes_per_row((WIDTH * 4) as usize) as u32;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("Luna MCP readback"),
        size: u64::from(row * HEIGHT),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        gpu.texture.as_image_copy(),
        ImageCopyBuffer {
            buffer: &buffer,
            layout: ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let mapped = buffer.clone();
    buffer.slice(..).map_async(MapMode::Read, move |result| {
        if let Err(error) = result {
            let _ = reply.send(Err(error.to_string()));
            return;
        }
        let data = {
            let bytes = mapped.slice(..).get_mapped_range();
            bytes
                .chunks(row as usize)
                .flat_map(|r| r[..(WIDTH * 4) as usize].iter().copied())
                .collect()
        };
        mapped.unmap();
        let image = Image::new(
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        encode(image, reply);
    });
}

pub struct AgentCapturePlugin;
impl Plugin for AgentCapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Readback>()
            .add_plugins(ExtractResourcePlugin::<Readback>::default())
            .add_systems(Startup, setup)
            .add_systems(PostUpdate, idle_camera);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                Render,
                readback_frame
                    .after(RenderSet::Render)
                    .before(RenderSet::Cleanup),
            );
        }
    }
}
