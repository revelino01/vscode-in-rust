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
pub mod process;
pub mod profiler;
pub mod provider;
pub mod rpc;
pub mod runtime_health;
pub mod scm;
pub mod secrets;
pub mod tasks;
pub mod terminal;
pub mod testing;
pub mod vfs;
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
pub use env::{
    is_valid_external_uri, ClipboardBackend, EnvEventListener, ExtEnvironmentInfo, ExtEnvEvent,
    ExtHostEnvManager, ExternalUriOpener, InMemoryClipboardBackend, OsClipboardBackend,
    OsExternalUriOpener, RecordingExternalUriOpener,
};
pub use manifest::{
    AuthenticationContribution, ContributedCommand, Contributions, CustomEditorContribution,
    CustomEditorSelector, DebuggerContribution, ExtensionLoader, ExtensionManifest,
    ExtensionSnapshot, GrammarContribution, IconThemeContribution, JsonValidationContribution,
    JsonValidationEntry, JsonValidationFileMatch, JsonValidationRegistry, KeybindingContribution,
    LanguageContribution, LoadedExtension, MenuContribution, ProductIconThemeContribution,
    SnippetContribution, SubmenuContribution, TaskDefinitionContribution, ThemeContribution,
    ViewContainerContribution, ViewContribution, ViewWelcomeContribution,
};
pub use marketplace::{
    curated_catalog, is_newer_version, search_curated_catalog, CurlHttpTransport,
    ExtensionAutoUpdater, ExtensionDownloader, ExtensionInstaller, ExtensionMetadata,
    ExtensionFiles, ExtensionUpdateAvailable, HttpTransport, InstalledExtensionMeta,
    MarketplaceClient, MarketplaceError, MockHttpTransport, SearchResponse, SemverVersion,
    WorkspaceExtensionsRecommendations,
};
pub use notebook::{
    ExtHostNotebookManager, ExtNotebookCellData, ExtNotebookCellKind, ExtNotebookCellOutput,
    ExtNotebookCellOutputItem, ExtNotebookData, NotebookCellExecutionState,
    NotebookControllerInfo, NotebookEventListener, NotebookSerializerInfo,
};
pub use output::{
    ExtHostOutputManager, ExtLogLevel, OutputChannelEvent, OutputChannelState, OutputEventListener,
};
pub use profiler::{
    ExtHostMemoryStats, ExtHostProfileSnapshot, ExtHostProfiler, ExtensionActivationProfile,
    RpcMethodMetrics,
};
pub use provider::{
    DiagnosticCollectionManager, ExtStatusBarItem, ExtStatusBarManager, ExtWorkspaceFolder,
    LanguageFeatureBridge, OpenDocumentSnapshot, ProviderRegistration, ProviderRegistry,
    ProviderType, TreeItem, TreeItemCheckbox, TreeItemCollapsibleState, TreeRefreshEvent,
    TreeViewOptionsMeta,
};
pub use rpc::{
    resolve_ext_host_entry, resolve_node_binary, NodeExtHostConfig, NodeExtHostProcess,
    RpcBridge, RpcError, RpcErrorPayload, RpcMessage, RpcRequestHandler, RpcWireFormat,
};
pub use runtime_health::{
    inspect_node_runtime, parse_node_version, ExtHostWatchdog, ManagedNodeStatus, NodeVersion,
    WatchdogHealth, DEFAULT_HEARTBEAT_INTERVAL, DEFAULT_MAX_AUTO_RESTARTS,
    DEFAULT_UNRESPONSIVE_THRESHOLD, MINIMUM_NODE_MAJOR,
};
pub use scm::{
    git_status_badge, parse_git_status_kind, ExtHostScmManager, ExtScmEvent, ExtScmInputBoxState,
    ExtScmResourceGroupState, ExtScmResourceState, ExtSourceControlState, ScmEventListener,
};
pub use secrets::{ExtHostSecretManager, SecretChangeEvent, SecretChangeListener};
pub use tasks::{
    ExtHostTaskManager, ExtTaskEvent, ExtTaskExecutionState, ExtTaskProviderInfo,
    ProblemMatcherContribution, ProblemMatcherRegistry, ProblemPattern, ProblemPatternSpec,
    TaskEventListener,
};
pub use terminal::{
    ExtHostTerminalManager, ExtTerminalEvent, ExtTerminalShellExecutionRecord,
    ExtTerminalShellIntegrationInfo, ExtTerminalState, TerminalEventListener,
};
pub use testing::{
    ExtHostTestManager, ExtTestControllerState, ExtTestEvent, ExtTestItemState,
    ExtTestResultRecord, ExtTestRunProfileInfo, ExtTestRunState, TestEventListener,
};
pub use vfs::{
    extract_uri_scheme, ExtHostVfsManager, RegisteredFileSystemProvider, VfsEvent,
    VfsEventListener, VfsFileChange,
};
pub use webview::{
    CustomEditorEvent, CustomEditorEventListener, CustomEditorProviderInfo,
    CustomEditorSessionState, ExtHostCustomEditorManager, ExtHostWebviewManager,
    ExtHostWebviewViewManager, WebviewEvent, WebviewEventListener, WebviewMessageListener,
    WebviewOptions, WebviewPanelState, WebviewViewEvent, WebviewViewEventListener,
    WebviewViewProviderInfo, WebviewViewState,
};
pub use window::{
    ExtHostWindowManager, ExtNotificationState, ExtPromptKind, ExtPromptState, ExtSelection,
    ExtTextEditorState, ExtWindowEvent, NotificationActionItem, NotificationResponder,
    NotificationSeverity, PromptResponder, ShowTextDocumentRequest, WindowEventListener,
};
