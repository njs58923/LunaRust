use std::{env, fs, path::{Path, PathBuf}};

/// Sube por los ancestros desde `start` hasta encontrar un Cargo.toml
fn find_cargo_root_from(start: &Path) -> Option<PathBuf> {
    for anc in start.ancestors() {
        let cand = anc.join("Cargo.toml");
        if cand.is_file() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

/// Busca Cargo.toml desde current_dir y desde la carpeta del ejecutable.
fn find_cargo_root() -> Option<PathBuf> {
    if let Ok(cwd) = env::current_dir() {
        if let Some(root) = find_cargo_root_from(&cwd) {
            return Some(root);
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(root) = find_cargo_root_from(dir) {
                return Some(root);
            }
        }
    }
    None
}

/// Devuelve:
/// - `assets_dir`: carpeta "assets" que usará Bevy como raíz
/// - `cache_dir`:  carpeta "assets/cache" donde guardamos archivos
pub fn resolve_assets_and_cache_dirs() -> (PathBuf, PathBuf) {
    // Si hay Cargo.toml => dev: usamos <root>/assets
    if let Some(root) = find_cargo_root() {
        let assets_dir = root.join("crates\\bevy_openxr\\assets");
        let cache_dir  = assets_dir.join("cache");
        return (assets_dir, cache_dir);
    }

    // Producción/binario: junto al ejecutable: <exe_dir>/assets
    let base = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| env::current_dir().unwrap());

    let assets_dir = base.join("assets");
    let cache_dir  = assets_dir.join("cache");
    (assets_dir, cache_dir)
}

/// Convierte una ruta absoluta bajo `assets_dir` a una ruta **relativa a assets/**
/// para poder usar `AssetServer::load("cache/archivo.glb#Scene0")`.
pub fn to_assets_relative(abs_path: &Path, assets_dir: &Path) -> Option<String> {
    abs_path.strip_prefix(assets_dir).ok().map(|p| p.to_string_lossy().to_string())
}
