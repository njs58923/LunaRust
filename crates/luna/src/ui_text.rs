use bevy::prelude::*;
use std::collections::HashMap;
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
                h.clone()
            } else {
                world.resource_mut::<Assets<Image>>().add(image)
            };
            cache.pages.insert(page, (revision, handle));
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
