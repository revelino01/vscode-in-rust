//! VSIX package downloader and zip archive unpacker.

use super::types::MarketplaceError;
use crate::manifest::ExtensionManifest;
use std::path::{Path, PathBuf};

pub struct ExtensionDownloader;

impl ExtensionDownloader {
    /// Resolve default user extensions directory (`~/.rustcode/extensions/`).
    pub fn default_extensions_dir() -> PathBuf {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()))
            .map(PathBuf::from)
            .or_else(|| {
                let drive = std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty())?;
                let path = std::env::var_os("HOMEPATH").filter(|v| !v.is_empty())?;
                let mut combined = drive;
                combined.push(path);
                Some(PathBuf::from(combined))
            });
        if let Some(home) = home {
            home.join(".rustcode").join("extensions")
        } else {
            PathBuf::from(".rustcode").join("extensions")
        }
    }

    /// Read and parse extension manifest (`package.json`) directly from in-memory VSIX package bytes.
    pub fn read_manifest_from_vsix(vsix_bytes: &[u8]) -> Result<ExtensionManifest, MarketplaceError> {
        let reader = std::io::Cursor::new(vsix_bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| MarketplaceError::Archive(format!("Failed to read zip archive: {}", e)))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| MarketplaceError::Archive(format!("Failed to read entry {}: {}", i, e)))?;

            if let Some(enclosed) = file.enclosed_name() {
                if enclosed == Path::new("extension/package.json") || enclosed == Path::new("package.json") {
                    let mut content = String::new();
                    std::io::Read::read_to_string(&mut file, &mut content)?;
                    return crate::manifest::ExtensionLoader::parse_manifest(&content)
                        .map_err(MarketplaceError::InvalidManifest);
                }
            }
        }

        Err(MarketplaceError::NotFound("package.json not found in VSIX archive".to_string()))
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
            if dest_rel.as_os_str().is_empty() {
                continue;
            }

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
