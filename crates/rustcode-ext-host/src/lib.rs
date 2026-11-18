//! `rustcode-ext-host` — Layer 4: Extension Host Subsystem.
//!
//! Manages VS Code extension discovery, manifest parsing (`package.json`),
//! MessagePack binary RPC communication with the JS runtime, and provider callbacks.

pub mod manifest;
pub mod marketplace;
pub mod notebook;
pub mod provider;
pub mod rpc;
pub mod webview;

pub use manifest::{
    ContributedCommand, Contributions, ExtensionLoader, ExtensionManifest, GrammarContribution,
    KeybindingContribution, LanguageContribution, LoadedExtension, ThemeContribution,
    ViewContribution,
};
pub use marketplace::{
    curated_catalog, search_curated_catalog, CurlHttpTransport, ExtensionDownloader,
    ExtensionFiles, ExtensionInstaller, ExtensionMetadata, HttpTransport, MarketplaceClient,
    MarketplaceError, MockHttpTransport, SearchResponse,
};
pub use notebook::{
    ExtHostNotebookManager, ExtNotebookCellData, ExtNotebookCellKind, ExtNotebookCellOutput,
    ExtNotebookCellOutputItem, ExtNotebookData, NotebookCellExecutionState,
    NotebookControllerInfo, NotebookEventListener, NotebookSerializerInfo,
};
pub use provider::{
    DiagnosticCollectionManager, ExtStatusBarItem, ExtStatusBarManager, LanguageFeatureBridge,
    ProviderRegistration, ProviderRegistry, ProviderType, TreeItem, TreeItemCollapsibleState,
};
pub use rpc::{RpcBridge, RpcMessage};
pub use webview::{
    ExtHostWebviewManager, WebviewMessageListener, WebviewOptions, WebviewPanelState,
};
