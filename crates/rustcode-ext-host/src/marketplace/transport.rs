//! HTTP transport abstraction supporting pluggable implementations and mock testing.

use super::types::MarketplaceError;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait HttpTransport: Send + Sync {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, MarketplaceError>>;
}

/// In-memory mock HTTP transport for deterministic testing without network dependencies.
#[derive(Default)]
pub struct MockHttpTransport {
    responses: Mutex<HashMap<String, Result<Vec<u8>, String>>>,
}

impl MockHttpTransport {
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_response(&self, url: impl Into<String>, body: impl Into<Vec<u8>>) {
        self.responses
            .lock()
            .insert(url.into(), Ok(body.into()));
    }

    pub fn set_error(&self, url: impl Into<String>, error_message: impl Into<String>) {
        self.responses
            .lock()
            .insert(url.into(), Err(error_message.into()));
    }
}

impl HttpTransport for MockHttpTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, MarketplaceError>> {
        let guard = self.responses.lock();
        let res = match guard.get(url) {
            Some(Ok(bytes)) => Ok(bytes.clone()),
            Some(Err(err)) => Err(MarketplaceError::Http(err.clone())),
            None => Err(MarketplaceError::NotFound(format!("Mock URL not mapped: {}", url))),
        };
        Box::pin(std::future::ready(res))
    }
}

/// Lightweight Curl-backed HTTP transport utilizing pre-installed `curl` utility.
pub struct CurlHttpTransport;

impl CurlHttpTransport {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CurlHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport for CurlHttpTransport {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, MarketplaceError>> {
        let url_owned = url.to_string();
        Box::pin(async move {
            let output = tokio::process::Command::new("curl")
                .args(["-sSL", "--fail", "--connect-timeout", "15", "-m", "120", &url_owned])
                .output()
                .await
                .map_err(|e| MarketplaceError::Http(format!("Failed to execute curl: {}", e)))?;

            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                return Err(MarketplaceError::Http(format!(
                    "curl exited with code {:?}: {}",
                    output.status.code(),
                    err_msg
                )));
            }

            Ok(output.stdout)
        })
    }
}
