//! `rustcode-ext-host` — Layer 4: Extension Host Subsystem.
//!
//! Manages VS Code extension discovery, manifest parsing (`package.json`),
//! MessagePack binary RPC communication with the JS runtime, and provider callbacks.

pub mod auth;
pub mod debug;
pub mod env;
pub mod manifest;
pub mod marketplace;
pub mod notebook;
pub mod output;
pub mod provider;
pub mod rpc;
pub mod scm;
pub mod tasks;
pub mod terminal;
pub mod testing;
pub mod webview;
pub mod window;

pub use auth::{
    AuthEventListener, ExtAuthAccountInfo, ExtAuthEvent, ExtAuthProviderInfo, ExtAuthSessionInfo,
    ExtHostAuthManager,
};
pub use debug::{
    DebugEventListener, ExtDebugBreakpoint, ExtDebugConsoleEntry, ExtDebugCustomEvent,
    ExtDebugEvent, ExtDebugProviderInfo, ExtDebugSessionState, ExtHostDebugManager,
};
pub use env::{EnvEventListener, ExtEnvEvent, ExtHostEnvManager};
pub use manifest::{
    AuthenticationContribution, ContributedCommand, Contributions, CustomEditorContribution,
    CustomEditorSelector, DebuggerContribution, ExtensionLoader, ExtensionManifest,
    ExtensionSnapshot, GrammarContribution, KeybindingContribution, LanguageContribution,
    LoadedExtension, TaskDefinitionContribution, ThemeContribution, ViewContribution,
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
pub use output::{
    ExtHostOutputManager, OutputChannelEvent, OutputChannelState, OutputEventListener,
};
pub use provider::{
    DiagnosticCollectionManager, ExtStatusBarItem, ExtStatusBarManager, LanguageFeatureBridge,
    ProviderRegistration, ProviderRegistry, ProviderType, TreeItem, TreeItemCollapsibleState,
};
pub use rpc::{RpcBridge, RpcMessage};
pub use scm::{
    git_status_badge, parse_git_status_kind, ExtHostScmManager, ExtScmEvent, ExtScmInputBoxState,
    ExtScmResourceGroupState, ExtScmResourceState, ExtSourceControlState, ScmEventListener,
};
pub use tasks::{
    ExtHostTaskManager, ExtTaskEvent, ExtTaskExecutionState, ExtTaskProviderInfo,
    TaskEventListener,
};
pub use terminal::{
    ExtHostTerminalManager, ExtTerminalEvent, ExtTerminalState, TerminalEventListener,
};
pub use testing::{
    ExtHostTestManager, ExtTestControllerState, ExtTestEvent, ExtTestItemState,
    ExtTestResultRecord, ExtTestRunProfileInfo, ExtTestRunState, TestEventListener,
};
pub use webview::{
    CustomEditorEvent, CustomEditorEventListener, CustomEditorProviderInfo,
    CustomEditorSessionState, ExtHostCustomEditorManager, ExtHostWebviewManager, WebviewEvent,
    WebviewEventListener, WebviewMessageListener, WebviewOptions, WebviewPanelState,
};
pub use window::{
    ExtHostWindowManager, ExtNotificationState, ExtPromptKind, ExtPromptState, ExtWindowEvent,
    NotificationActionItem, NotificationResponder, NotificationSeverity, PromptResponder,
    WindowEventListener,
};
