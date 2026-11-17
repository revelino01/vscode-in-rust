//! VSIX package downloader and zip archive unpacker.

use super::types::MarketplaceError;
use crate::manifest::ExtensionManifest;
use std::path::{Path, PathBuf};

pub struct ExtensionDownloader;

impl ExtensionDownloader {
    /// Resolve default user extensions directory (`~/.rustcode/extensions/`).
    pub fn default_extensions_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".rustcode").join("extensions")
        } else {
            PathBuf::from(".rustcode").join("extensions")
        }
    }

    /// Unpack a `.vsix` ZIP archive into `target_dir` and parse the root `package.json`.
    /// Automatically strips the standard `"extension/"` package root prefix.
    pub fn unpack_vsix(vsix_bytes: &[u8], target_dir: &Path) -> Result<ExtensionManifest, MarketplaceError> {
        std::fs::create_dir_all(target_dir)?;

        let reader = std::io::Cursor::new(vsix_bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| MarketplaceError::Archive(format!("Failed to read zip archive: {}", e)))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| MarketplaceError::Archive(format!("Failed to read entry {}: {}", i, e)))?;

            let enclosed = match file.enclosed_name() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            // Strip "extension" prefix if present as VSIX packages place contents in `extension/`
            let dest_rel = enclosed.strip_prefix("extension").unwrap_or(&enclosed);

            let dest = target_dir.join(dest_rel);

            if file.is_dir() {
                std::fs::create_dir_all(&dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = std::fs::File::create(&dest)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }

        let package_json_path = target_dir.join("package.json");
        if !package_json_path.exists() {
            return Err(MarketplaceError::NotFound(
                "package.json not found in unpacked extension".to_string(),
            ));
        }

        let manifest_content = std::fs::read_to_string(&package_json_path)?;
        crate::manifest::ExtensionLoader::parse_manifest(&manifest_content)
            .map_err(MarketplaceError::InvalidManifest)
    }
}
