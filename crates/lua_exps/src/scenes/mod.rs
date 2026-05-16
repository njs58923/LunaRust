use bevy::prelude::*;

pub mod default_scene;
pub mod lua_rock;
pub mod spinning_cube;

#[derive(States, Debug, Clone, Eq, PartialEq, Hash, Default)]
pub enum SceneId {
    #[default]
    Default,
    SpinningCube,
    LuaRock,
}

impl SceneId {
    pub const ALL: &'static [SceneId] = &[
        SceneId::Default,
        SceneId::SpinningCube,
        SceneId::LuaRock,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SceneId::Default => "1. Default",
            SceneId::SpinningCube => "2. Spinning Cube",
            SceneId::LuaRock => "3. Lua Rock",
        }
    }
}

#[derive(Component)]
pub struct SceneRoot;

pub struct ScenesPlugin;

impl Plugin for ScenesPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<SceneId>()
            .init_resource::<lua_rock::RockState>()
            .init_resource::<lua_rock::RockCache>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, (scene_selector, update_hud))
            .add_systems(OnExit(SceneId::Default), despawn_scene)
            .add_systems(OnExit(SceneId::SpinningCube), despawn_scene)
            .add_systems(OnExit(SceneId::LuaRock), despawn_scene)
            .add_systems(OnEnter(SceneId::Default), default_scene::setup)
            .add_systems(OnEnter(SceneId::SpinningCube), spinning_cube::setup)
            .add_systems(OnEnter(SceneId::LuaRock), lua_rock::setup)
            .add_systems(
                Update,
                spinning_cube::rotate.run_if(in_state(SceneId::SpinningCube)),
            )
            .add_systems(
                Update,
                (lua_rock::input, lua_rock::regenerate, lua_rock::update_hud)
                    .chain()
                    .run_if(in_state(SceneId::LuaRock)),
            );
    }
}

fn despawn_scene(mut commands: Commands, q: Query<Entity, With<SceneRoot>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn scene_selector(
    keys: Res<ButtonInput<KeyCode>>,
    current: Res<State<SceneId>>,
    mut next: ResMut<NextState<SceneId>>,
) {
    let pick = if keys.just_pressed(KeyCode::Digit1) {
        Some(SceneId::Default)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(SceneId::SpinningCube)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(SceneId::LuaRock)
    } else {
        None
    };
    if let Some(s) = pick {
        if *current.get() != s {
            next.set(s);
        }
    }
}

#[derive(Component)]
struct HudText;

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        TextBundle::from_section(
            "",
            TextStyle {
                font_size: 22.0,
                color: Color::WHITE,
                ..default()
            },
        )
        .with_style(Style {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        }),
        HudText,
    ));
}

fn update_hud(state: Res<State<SceneId>>, mut q: Query<&mut Text, With<HudText>>) {
    let Ok(mut text) = q.get_single_mut() else {
        return;
    };
    let mut s = String::from("Scenes (press 1-3):\n");
    for id in SceneId::ALL {
        let marker = if id == state.get() { "> " } else { "  " };
        s.push_str(&format!("{}{}\n", marker, id.label()));
    }
    text.sections[0].value = s;
}
