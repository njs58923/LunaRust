use std::{env, path::{Path, PathBuf}};

fn find_cargo_root_from(start: &Path) -> Option<PathBuf> {
    for anc in start.ancestors() {
        if anc.join("Cargo.toml").is_file() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

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

/// Returns:
/// - `assets_dir`: Bevy asset root
/// - `cache_dir`:  `assets/cache` for downloaded files
pub fn resolve_assets_and_cache_dirs() -> (PathBuf, PathBuf) {
    if let Some(root) = find_cargo_root() {
        let assets_dir = root.join("crates/luna/assets");
        let cache_dir = assets_dir.join("cache");
        return (assets_dir, cache_dir);
    }

    let base = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| env::current_dir().unwrap());

    let assets_dir = base.join("assets");
    let cache_dir = assets_dir.join("cache");
    (assets_dir, cache_dir)
}

pub fn to_assets_relative(abs_path: &Path, assets_dir: &Path) -> Option<String> {
    abs_path.strip_prefix(assets_dir).ok().map(|p| p.to_string_lossy().to_string())
}
