//! `rustcode-ext-host` — Layer 4: Extension Host Subsystem.
//!
//! Manages VS Code extension discovery, manifest parsing (`package.json`),
//! MessagePack binary RPC communication with the JS runtime, and provider callbacks.

pub mod manifest;
pub mod marketplace;
pub mod provider;
pub mod rpc;

pub use manifest::{
    ContributedCommand, Contributions, ExtensionLoader, ExtensionManifest, GrammarContribution,
    KeybindingContribution, LanguageContribution, LoadedExtension, ThemeContribution,
};
pub use marketplace::{
    CurlHttpTransport, ExtensionDownloader, ExtensionFiles, ExtensionInstaller, ExtensionMetadata,
    HttpTransport, MarketplaceClient, MarketplaceError, MockHttpTransport, SearchResponse,
};
pub use provider::{
    DiagnosticCollectionManager, LanguageFeatureBridge, ProviderRegistration, ProviderRegistry,
    ProviderType,
};
pub use rpc::{RpcBridge, RpcMessage};
