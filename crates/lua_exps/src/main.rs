mod scenes;

use bevy::prelude::*;
use mlua::Lua;
use scenes::ScenesPlugin;

#[derive(Resource)]
struct LuaVm(Lua);

#[derive(Resource, Default)]
struct Frame(u64);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ScenesPlugin)
        .insert_resource(LuaVm(Lua::new()))
        .insert_resource(Frame::default())
        .add_systems(Startup, lua_startup)
        .add_systems(Update, lua_tick)
        .run();
}

fn lua_startup(lua: Res<LuaVm>) {
    lua.0
        .load(r#"print("[lua] hello from startup")"#)
        .exec()
        .expect("lua startup script failed");
}

fn lua_tick(lua: Res<LuaVm>, mut frame: ResMut<Frame>) {
    frame.0 += 1;
    if frame.0 % 60 != 0 {
        return;
    }
    let globals = lua.0.globals();
    globals.set("frame", frame.0).unwrap();
    lua.0
        .load(r#"print(string.format("[lua] tick frame=%d", frame))"#)
        .exec()
        .expect("lua tick script failed");
}
