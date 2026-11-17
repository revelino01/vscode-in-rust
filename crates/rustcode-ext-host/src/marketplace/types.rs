//! Open VSX Marketplace data structures and error types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MarketplaceError {
    #[error("HTTP request error: {0}")]
    Http(String),
    #[error("Serialization / JSON error: {0}")]
    Serialization(String),
    #[error("Archive error: {0}")]
    Archive(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Extension not found: {0}")]
    NotFound(String),
    #[error("Invalid extension manifest: {0}")]
    InvalidManifest(String),
    #[error("Missing download URL for extension: {0}")]
    MissingDownloadUrl(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionFiles {
    pub download: Option<String>,
    pub icon: Option<String>,
    pub manifest: Option<String>,
    pub readme: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionMetadata {
    pub namespace: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "downloadUrl")]
    pub download_url: Option<String>,
    pub files: Option<ExtensionFiles>,
    #[serde(rename = "allVersions", default)]
    pub all_versions: Option<HashMap<String, String>>,
}

impl ExtensionMetadata {
    pub fn resolved_download_url(&self) -> Option<String> {
        if let Some(url) = &self.download_url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
        if let Some(files) = &self.files {
            if let Some(url) = &files.download {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    #[serde(rename = "totalSize", default)]
    pub total_size: usize,
    #[serde(default)]
    pub extensions: Vec<ExtensionMetadata>,
}
