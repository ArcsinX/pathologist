use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn discover_c_files(root: &Path) -> Vec<PathBuf> {
    discover_by_extension(root, "c")
}

/// `.h` files under the analyzed tree (for struct layouts), not external include dirs.
pub fn discover_header_files(root: &Path) -> Vec<PathBuf> {
    discover_by_extension(root, "h")
}

fn discover_by_extension(root: &Path, ext: &str) -> Vec<PathBuf> {
    if root.is_file() {
        return root
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| *e == ext)
            .map(|_| vec![root.to_path_buf()])
            .unwrap_or_default();
    }
    let mut paths: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == ext)
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    paths.sort();
    paths
}
