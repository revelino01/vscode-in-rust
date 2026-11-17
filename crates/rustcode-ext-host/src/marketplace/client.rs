//! Open VSX REST API client.

use super::transport::{CurlHttpTransport, HttpTransport};
use super::types::{ExtensionMetadata, MarketplaceError, SearchResponse};
use std::sync::Arc;

pub struct MarketplaceClient {
    base_url: String,
    transport: Arc<dyn HttpTransport>,
}

impl Default for MarketplaceClient {
    fn default() -> Self {
        Self::default_client()
    }
}

impl MarketplaceClient {
    pub fn new(base_url: impl Into<String>, transport: Arc<dyn HttpTransport>) -> Self {
        let mut base = base_url.into();
        if base.ends_with('/') {
            base.pop();
        }
        Self {
            base_url: base,
            transport,
        }
    }

    pub fn default_client() -> Self {
        Self::new("https://open-vsx.org/api", Arc::new(CurlHttpTransport::new()))
    }

    /// Query Open VSX search endpoint by keyword.
    pub async fn search(
        &self,
        query: &str,
        size: Option<usize>,
        offset: Option<usize>,
    ) -> Result<SearchResponse, MarketplaceError> {
        let size_param = size.unwrap_or(20);
        let offset_param = offset.unwrap_or(0);
        let encoded_query = url_encode(query);
        let url = format!(
            "{}/-/search?query={}&size={}&offset={}",
            self.base_url, encoded_query, size_param, offset_param
        );

        let bytes = self.transport.get(&url).await?;
        serde_json::from_slice::<SearchResponse>(&bytes)
            .map_err(|e| MarketplaceError::Serialization(format!("SearchResponse parse failed: {}", e)))
    }

    /// Fetch latest extension metadata by namespace and extension name.
    pub async fn get_extension_metadata(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ExtensionMetadata, MarketplaceError> {
        let url = format!("{}/{}/{}", self.base_url, namespace, name);
        let bytes = self.transport.get(&url).await?;
        serde_json::from_slice::<ExtensionMetadata>(&bytes)
            .map_err(|e| MarketplaceError::Serialization(format!("ExtensionMetadata parse failed: {}", e)))
    }

    /// Fetch specific version metadata for an extension.
    pub async fn get_version_metadata(
        &self,
        namespace: &str,
        name: &str,
        version: &str,
    ) -> Result<ExtensionMetadata, MarketplaceError> {
        let url = format!("{}/{}/{}/{}", self.base_url, namespace, name, version);
        let bytes = self.transport.get(&url).await?;
        serde_json::from_slice::<ExtensionMetadata>(&bytes)
            .map_err(|e| MarketplaceError::Serialization(format!("Version metadata parse failed: {}", e)))
    }

    /// Download `.vsix` archive package bytes from a given URL.
    pub async fn download_vsix(&self, download_url: &str) -> Result<Vec<u8>, MarketplaceError> {
        self.transport.get(download_url).await
    }
}

fn url_encode(input: &str) -> String {
    let mut encoded = String::new();
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}
