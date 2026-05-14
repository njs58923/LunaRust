use std::{
    env,
    path::{Path, PathBuf},
};

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
/// - `assets_dir`: Bevy asset root (CWD-relative, matches `AssetPlugin.file_path = "assets"`)
/// - `cache_dir`:  `assets/cache` for downloaded files
pub fn resolve_assets_and_cache_dirs() -> (PathBuf, PathBuf) {
    let base = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let assets_dir = base.join("assets");
    let cache_dir = assets_dir.join("cache");
    (assets_dir, cache_dir)
}

pub fn resolve_luna_state_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("LunaBrowser");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg_state_home) = env::var("XDG_STATE_HOME") {
            return PathBuf::from(xdg_state_home).join("luna");
        }
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("luna");
        }
    }

    if let Some(root) = find_cargo_root() {
        return root.join(".luna");
    }

    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".luna")
}

pub fn resolve_root_config_path() -> PathBuf {
    resolve_luna_state_dir().join("root-config.json")
}

pub fn to_assets_relative(abs_path: &Path, assets_dir: &Path) -> Option<String> {
    abs_path
        .strip_prefix(assets_dir)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn to_assets_relative_strips_prefix() {
        let assets = PathBuf::from("/project/assets");
        let abs = PathBuf::from("/project/assets/cache/model.glb");
        let rel = to_assets_relative(&abs, &assets).unwrap();
        assert_eq!(rel, "cache/model.glb");
    }

    #[test]
    fn to_assets_relative_wrong_prefix_returns_none() {
        let assets = PathBuf::from("/project/assets");
        let abs = PathBuf::from("/other/path/model.glb");
        assert!(to_assets_relative(&abs, &assets).is_none());
    }

    #[test]
    fn to_assets_relative_exact_match_returns_empty() {
        let assets = PathBuf::from("/project/assets");
        let result = to_assets_relative(&assets, &assets).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn resolve_assets_dirs_returns_paths() {
        let (assets_dir, cache_dir) = resolve_assets_and_cache_dirs();
        assert!(assets_dir.ends_with("assets"));
        assert!(cache_dir.ends_with("cache"));
        assert!(cache_dir.starts_with(&assets_dir));
    }

    #[test]
    fn root_config_path_ends_with_expected_filename() {
        assert!(resolve_root_config_path().ends_with("root-config.json"));
    }
}
