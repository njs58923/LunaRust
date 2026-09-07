//! CPU-only reproduction of the redundant camera visibility work, not an FPS test.
use bevy::{prelude::*, render::{primitives::{Aabb, Frustum}, view::{check_visibility, VisibleEntities}}};

fn scene(count: usize) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_systems(Update, check_visibility::<With<Handle<Mesh>>>);
    let desktop = app.world_mut().spawn(Camera3dBundle {
        frustum: Frustum::from_clip_from_world(&Mat4::perspective_rh(1.05, 2000.0/1041.0, 0.1, 500.0)), ..default() }).id();
    let overlay = app.world_mut().spawn(Camera2dBundle {
        frustum: Frustum::from_clip_from_world(&Mat4::orthographic_rh(-1000., 1000., -520.5, 520.5, -1000., 1000.)), ..default() }).id();
    for i in 0..count {
        app.world_mut().spawn((Handle::<Mesh>::default(), InheritedVisibility::VISIBLE,
            ViewVisibility::default(), Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
            GlobalTransform::from_xyz((i % 200) as f32 - 100., ((i / 200) % 5) as f32, -5. - (i / 1000) as f32 * 3.)));
    }
    (app, desktop, overlay)
}

#[test]
fn removing_overlay_preserves_desktop_visible_meshes() {
    let (mut app, desktop, overlay) = scene(5000);
    app.update();
    let mut before = app.world().get::<VisibleEntities>(desktop).unwrap().get::<With<Handle<Mesh>>>().to_vec();
    assert!(app.world().get::<VisibleEntities>(overlay).unwrap().len::<With<Handle<Mesh>>>() > 0);
    app.world_mut().despawn(overlay);
    app.update();
    let mut after = app.world().get::<VisibleEntities>(desktop).unwrap().get::<With<Handle<Mesh>>>().to_vec();
    before.sort(); after.sort();
    assert_eq!(before, after);
}

#[test]
#[ignore = "manual CPU benchmark; no GPU/window, no FPS claim"]
fn benchmark_dense_visibility() {
    for count in [30_085, 76_779] {
        let (mut app, desktop, overlay) = scene(count);
        for _ in 0..20 { app.update(); }
        let mut one_view = Vec::new();
        let mut two_views = Vec::new();
        for sample in 0..160 {
            // Alternate order to reduce drift from warmup and scheduling.
            for active in if sample % 2 == 0 { [true, false] } else { [false, true] } {
                app.world_mut().get_mut::<Camera>(overlay).unwrap().is_active = active;
                let start = std::time::Instant::now(); app.update();
                let elapsed = start.elapsed().as_secs_f64() * 1000.;
                if active { two_views.push(elapsed); } else { one_view.push(elapsed); }
            }
        }
        one_view.sort_by(f64::total_cmp); two_views.sort_by(f64::total_cmp);
        println!("primitives={count} desktop_visible={} one_view_median_ms={:.4} two_views_median_ms={:.4}",
            app.world().get::<VisibleEntities>(desktop).unwrap().len::<With<Handle<Mesh>>>(), one_view[80], two_views[80]);
    }
}
