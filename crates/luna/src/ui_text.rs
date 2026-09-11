use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
pub use ui_graphics::layout;
use ui_graphics::SIDE;
#[derive(Resource, Default)]
pub struct AtlasImages {
    pages: HashMap<usize, (u64, Handle<Image>)>,
    epoch: u64,
}
pub fn sync(world: &mut World) {
    let epoch = ui_graphics::epoch();
    world.init_resource::<AtlasImages>();
    if world.resource::<AtlasImages>().epoch == epoch {
        return;
    }
    let revisions = world
        .resource::<AtlasImages>()
        .pages
        .iter()
        .map(|(i, (r, _))| (*i, *r))
        .collect();
    let pages = ui_graphics::changed_pages(&revisions);
    let mut replaced = HashSet::new();
    world.resource_scope(|world, mut cache: Mut<AtlasImages>| {
        for (page, revision, pixels) in pages {
            let image = Image::new(
                bevy::render::render_resource::Extent3d {
                    width: SIDE as u32,
                    height: SIDE as u32,
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                pixels,
                bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                bevy::render::render_asset::RenderAssetUsages::default(),
            );
            let handle = if let Some((_, h)) = cache.pages.get(&page) {
                world.resource_mut::<Assets<Image>>().insert(h.id(), image);
                replaced.insert(h.id());
                h.clone()
            } else {
                world.resource_mut::<Assets<Image>>().add(image)
            };
            cache.pages.insert(page, (revision, handle));
        }
        cache.epoch = epoch;
    });
    invalidate_atlas_materials(world, &replaced);
}
pub fn image(world: &mut World, page: usize) -> Option<Handle<Image>> {
    sync(world);
    world
        .resource::<AtlasImages>()
        .pages
        .get(&page)
        .map(|(_, h)| h.clone())
}

// Bevy 0.14 replaces GpuImage when Image changes, but prepared materials retain
// their previous texture bind group until the material itself changes. Without
// this invalidation newly allocated glyph slots remain transparent indefinitely.
fn invalidate_atlas_materials(world: &mut World, pages: &HashSet<bevy::asset::AssetId<Image>>) {
    if pages.is_empty() {
        return;
    }
    let Some(mut materials) = world.get_resource_mut::<Assets<crate::surface::SurfaceMaterial>>()
    else {
        return;
    };
    let affected: Vec<_> = materials
        .iter()
        .filter_map(|(id, material)| {
            material
                .extension
                .texture
                .as_ref()
                .filter(|image| pages.contains(&image.id()))
                .map(|_| id)
        })
        .collect();
    for id in affected {
        // get_mut queues AssetEvent::Modified, rebuilding the GPU bind group.
        let _ = materials.get_mut(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{SurfaceExtension, SurfaceMaterial, SurfaceUniform};

    #[test]
    fn atlas_growth_invalidates_only_materials_using_changed_pages() {
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<Assets<SurfaceMaterial>>()
            .add_event::<AssetEvent<SurfaceMaterial>>()
            .add_systems(Update, Assets::<SurfaceMaterial>::asset_events);
        let atlas = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let other = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::default());
        let make = |image| SurfaceMaterial {
            base: StandardMaterial::default(),
            extension: SurfaceExtension {
                settings: SurfaceUniform::default(),
                texture: Some(image),
            },
        };
        let a = app
            .world_mut()
            .resource_mut::<Assets<SurfaceMaterial>>()
            .add(make(atlas.clone()));
        let b = app
            .world_mut()
            .resource_mut::<Assets<SurfaceMaterial>>()
            .add(make(other));
        app.update();
        app.world_mut()
            .resource_mut::<Events<AssetEvent<SurfaceMaterial>>>()
            .clear();
        invalidate_atlas_materials(app.world_mut(), &HashSet::from([atlas.id()]));
        app.update();
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Events<AssetEvent<SurfaceMaterial>>>()
            .drain()
            .collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, AssetEvent::Modified { id } if *id == a.id())));
        assert!(!events
            .iter()
            .any(|e| matches!(e, AssetEvent::Modified { id } if *id == b.id())));
        invalidate_atlas_materials(app.world_mut(), &HashSet::new());
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Events<AssetEvent<SurfaceMaterial>>>()
                .drain()
                .count(),
            0
        );
    }
}
