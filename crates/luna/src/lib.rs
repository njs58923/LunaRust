// Luna - biblioteca interna
// Los módulos aquí son testeables sin depender de bevy_mod_openxr (openxr_sys).
// El binario (main.rs) importa desde aquí con `use luna::*`.

pub mod components;
pub mod ui_text;
pub mod audio;
pub mod capture;
mod profiling;
#[cfg(test)]
mod density_tests;
pub mod agent;
mod agent_capture;
pub mod desktop_locomotion;
pub mod dom;
pub mod embedded;
pub mod io;
mod http_fetch;
pub mod models;
pub mod diagnostics;
pub mod model_animation;
pub mod dynamic_mesh;
pub mod surface;
pub mod js;
pub mod permissions;
pub mod render;
pub mod routes;
pub mod system_input;
pub mod touch;
pub mod viewer_pose;
pub mod player_spawn;
pub mod types;
pub mod ui;
pub mod utils;
pub mod vr_locomotion;
pub mod avatar_vm;
pub mod ws;

// Re-exportar todos los tipos públicos para que main.rs pueda hacer `use luna::*`
pub use dom::*;
pub use io::*;
pub use permissions::*;
pub use render::{apply_transform, resolve_remote_path};
pub use routes::VIRTUAL_ROUTES;
pub use types::*;
pub use ws::*;
