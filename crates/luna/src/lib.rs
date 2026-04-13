// Luna - biblioteca interna
// Los módulos aquí son testeables sin depender de bevy_mod_openxr (openxr_sys).
// El binario (main.rs) importa desde aquí con `use luna::*`.

pub mod desktop_locomotion;
pub mod dom;
pub mod io;
pub mod js;
pub mod permissions;
pub mod render;
pub mod routes;
pub mod touch;
pub mod types;
pub mod ui;
pub mod utils;
pub mod vr_locomotion;
pub mod avatar_vm;

// Re-exportar todos los tipos públicos para que main.rs pueda hacer `use luna::*`
pub use dom::*;
pub use io::*;
pub use permissions::*;
pub use render::{apply_transform, resolve_remote_path};
pub use routes::VIRTUAL_ROUTES;
pub use types::*;
