use std::path::PathBuf;

#[cfg(not(target_family = "wasm"))]
fn store_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("pdf-reader-gpui").join("session.txt"))
}

#[cfg(not(target_family = "wasm"))]
pub fn load() -> Vec<PathBuf> {
    let Some(path) = store_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(target_family = "wasm"))]
pub fn save(paths: &[PathBuf]) {
    let Some(path) = store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        _ = std::fs::create_dir_all(parent);
    }
    let text = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    _ = std::fs::write(path, text);
}

#[cfg(target_family = "wasm")]
pub fn load() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_family = "wasm")]
pub fn save(_paths: &[PathBuf]) {}
