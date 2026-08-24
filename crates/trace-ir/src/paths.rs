//! Filesystem path helpers shared by all pipeline stages.

use std::path::{Path, PathBuf};

/// Canonicalize `path`, falling back to the original path on error, and strip
/// the Windows extended-length prefix (`\\?\C:\...`,
/// `\\?\UNC\server\share\...`) that `std::fs::canonicalize` adds. Without the
/// strip, every interned file name in the IR and the exported database would
/// carry the prefix on Windows, breaking display and substring matching.
pub fn canonicalize(path: &Path) -> PathBuf {
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return path.to_path_buf(),
    };
    #[cfg(windows)]
    {
        let text = canonical.as_os_str().to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    canonical
}
