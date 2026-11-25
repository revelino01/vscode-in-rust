use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

impl<'de> Deserialize<'de> for DiagnosticSeverity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SeverityVisitor;
        impl<'de> serde::de::Visitor<'de> for SeverityVisitor {
            type Value = DiagnosticSeverity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an integer severity (0-4) or severity name")
            }

            fn visit_u64<E>(self, value: u64) -> Result<DiagnosticSeverity, E>
            where
                E: serde::de::Error,
            {
                match value {
                    0 | 1 => Ok(DiagnosticSeverity::Error),
                    2 => Ok(DiagnosticSeverity::Warning),
                    3 => Ok(DiagnosticSeverity::Information),
                    4 => Ok(DiagnosticSeverity::Hint),
                    _ => Err(E::custom(format!("invalid diagnostic severity: {}", value))),
                }
            }

            fn visit_i64<E>(self, value: i64) -> Result<DiagnosticSeverity, E>
            where
                E: serde::de::Error,
            {
                self.visit_u64(value as u64)
            }

            fn visit_str<E>(self, value: &str) -> Result<DiagnosticSeverity, E>
            where
                E: serde::de::Error,
            {
                match value.to_ascii_lowercase().as_str() {
                    "error" => Ok(DiagnosticSeverity::Error),
                    "warning" => Ok(DiagnosticSeverity::Warning),
                    "information" | "info" => Ok(DiagnosticSeverity::Information),
                    "hint" => Ok(DiagnosticSeverity::Hint),
                    _ => Err(E::custom(format!("invalid diagnostic severity string: {}", value))),
                }
            }
        }

        deserializer.deserialize_any(SeverityVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

impl LspPosition {
    pub const fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

impl LspRange {
    pub const fn new(start: LspPosition, end: LspPosition) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspLocation {
    #[serde(alias = "targetUri")]
    pub uri: String,
    #[serde(alias = "targetSelectionRange", alias = "targetRange")]
    pub range: LspRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiagnosticCode {
    String(String),
    Integer(i64),
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnosticCode::String(s) => write!(f, "{}", s),
            DiagnosticCode::Integer(i) => write!(f, "{}", i),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: Option<DiagnosticSeverity>,
    pub message: String,
    pub source: Option<String>,
    #[serde(default)]
    pub code: Option<DiagnosticCode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishDiagnosticsParams {
    pub uri: String,
    #[serde(default)]
    pub version: Option<i32>,
    pub diagnostics: Vec<LspDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CompletionItemKind {
    Text = 1,
    Method = 2,
    Function = 3,
    Constructor = 4,
    Field = 5,
    Variable = 6,
    Class = 7,
    Interface = 8,
    Module = 9,
    Property = 10,
    Unit = 11,
    Value = 12,
    Enum = 13,
    Keyword = 14,
    Snippet = 15,
    Color = 16,
    File = 17,
    Reference = 18,
    Folder = 19,
    EnumMember = 20,
    Constant = 21,
    Struct = 22,
    Event = 23,
    Operator = 24,
    TypeParameter = 25,
}

impl CompletionItemKind {
    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::Text),
            2 => Some(Self::Method),
            3 => Some(Self::Function),
            4 => Some(Self::Constructor),
            5 => Some(Self::Field),
            6 => Some(Self::Variable),
            7 => Some(Self::Class),
            8 => Some(Self::Interface),
            9 => Some(Self::Module),
            10 => Some(Self::Property),
            11 => Some(Self::Unit),
            12 => Some(Self::Value),
            13 => Some(Self::Enum),
            14 => Some(Self::Keyword),
            15 => Some(Self::Snippet),
            16 => Some(Self::Color),
            17 => Some(Self::File),
            18 => Some(Self::Reference),
            19 => Some(Self::Folder),
            20 => Some(Self::EnumMember),
            21 => Some(Self::Constant),
            22 => Some(Self::Struct),
            23 => Some(Self::Event),
            24 => Some(Self::Operator),
            25 => Some(Self::TypeParameter),
            _ => None,
        }
    }
}

impl Serialize for CompletionItemKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for CompletionItemKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = u8::deserialize(deserializer)?;
        Self::from_u8(val).ok_or_else(|| serde::de::Error::custom(format!("invalid CompletionItemKind: {}", val)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LspCompletionItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<CompletionItemKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_documentation",
        skip_serializing_if = "Option::is_none"
    )]
    pub documentation: Option<String>,
    #[serde(rename = "insertText", alias = "insert_text", skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    #[serde(rename = "filterText", alias = "filter_text", skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    #[serde(rename = "sortText", alias = "sort_text", skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
}

pub fn deserialize_documentation<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Object(map)) => {
            if let Some(serde_json::Value::String(val)) = map.get("value") {
                Ok(Some(val.clone()))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

impl LspCompletionItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Default::default()
        }
    }

    pub fn with_kind(mut self, kind: CompletionItemKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_doc(mut self, doc: impl Into<String>) -> Self {
        self.documentation = Some(doc.into());
        self
    }

    pub fn with_insert_text(mut self, insert_text: impl Into<String>) -> Self {
        self.insert_text = Some(insert_text.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompletionList {
    #[serde(rename = "isIncomplete", default)]
    pub is_incomplete: bool,
    pub items: Vec<LspCompletionItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompletionTriggerKind {
    Invoked = 1,
    TriggerCharacter = 2,
    TriggerForIncompleteCompletions = 3,
}

impl CompletionTriggerKind {
    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::Invoked),
            2 => Some(Self::TriggerCharacter),
            3 => Some(Self::TriggerForIncompleteCompletions),
            _ => None,
        }
    }
}

impl Serialize for CompletionTriggerKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for CompletionTriggerKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = u8::deserialize(deserializer)?;
        Self::from_u8(val).ok_or_else(|| serde::de::Error::custom(format!("invalid CompletionTriggerKind: {}", val)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionContext {
    #[serde(rename = "triggerKind")]
    pub trigger_kind: CompletionTriggerKind,
    #[serde(rename = "triggerCharacter", skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkupKind {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkupContent {
    pub kind: MarkupKind,
    pub value: String,
}

impl MarkupContent {
    pub fn markdown(val: impl Into<String>) -> Self {
        Self {
            kind: MarkupKind::Markdown,
            value: val.into(),
        }
    }

    pub fn plain_text(val: impl Into<String>) -> Self {
        Self {
            kind: MarkupKind::PlainText,
            value: val.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hover {
    #[serde(deserialize_with = "deserialize_markup_content")]
    pub contents: MarkupContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<LspRange>,
}

pub fn deserialize_optional_markup_content<'de, D>(
    deserializer: D,
) -> Result<Option<MarkupContent>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(v) = val else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    Ok(Some(parse_markup_value(v)))
}

fn parse_markup_value(val: serde_json::Value) -> MarkupContent {
    match val {
        serde_json::Value::String(s) => MarkupContent::markdown(s),
        serde_json::Value::Object(map) => {
            let kind_str = map.get("kind").and_then(|k| k.as_str()).unwrap_or("markdown");
            let kind = if kind_str.eq_ignore_ascii_case("plaintext") {
                MarkupKind::PlainText
            } else {
                MarkupKind::Markdown
            };
            let value = map
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            MarkupContent { kind, value }
        }
        serde_json::Value::Array(arr) => {
            let mut combined = String::new();
            for item in arr {
                match item {
                    serde_json::Value::String(s) => {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(&s);
                    }
                    serde_json::Value::Object(map) => {
                        if let Some(val) = map.get("value").and_then(|v| v.as_str()) {
                            if !combined.is_empty() {
                                combined.push_str("\n\n");
                            }
                            if let Some(lang) = map.get("language").and_then(|l| l.as_str()) {
                                combined.push_str(&format!("```{}\n{}\n```", lang, val));
                            } else {
                                combined.push_str(val);
                            }
                        }
                    }
                    _ => {}
                }
            }
            MarkupContent::markdown(combined)
        }
        _ => MarkupContent::markdown(""),
    }
}

pub fn deserialize_markup_content<'de, D>(deserializer: D) -> Result<MarkupContent, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    Ok(parse_markup_value(val))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(rename = "languageId")]
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDocumentContentChangeEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<LspRange>,
    #[serde(rename = "rangeLength", skip_serializing_if = "Option::is_none")]
    pub range_length: Option<u32>,
    pub text: String,
}

impl TextDocumentContentChangeEvent {
    pub fn full(text: impl Into<String>) -> Self {
        Self {
            range: None,
            range_length: None,
            text: text.into(),
        }
    }

    pub fn incremental(range: LspRange, text: impl Into<String>) -> Self {
        Self {
            range: Some(range),
            range_length: None,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidOpenTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidChangeTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: VersionedTextDocumentIdentifier,
    #[serde(rename = "contentChanges")]
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidCloseTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(rename = "textDocument", alias = "text_document", default)]
    pub text_document: Option<serde_json::Value>,
    #[serde(default)]
    pub workspace: Option<serde_json::Value>,
}

impl ClientCapabilities {
    pub fn default_phase9() -> Self {
        Self {
            text_document: Some(serde_json::json!({
                "synchronization": {
                    "didSave": true,
                    "willSave": false
                },
                "completion": {
                    "completionItem": {
                        "snippetSupport": true,
                        "documentationFormat": ["markdown", "plaintext"]
                    }
                },
                "hover": {
                    "contentFormat": ["markdown", "plaintext"]
                },
                "signatureHelp": {
                    "signatureInformation": {
                        "documentationFormat": ["markdown", "plaintext"],
                        "parameterInformation": {
                            "labelOffsetSupport": true
                        },
                        "activeParameterSupport": true
                    }
                },
                "definition": { "linkSupport": true },
                "typeDefinition": { "linkSupport": true },
                "implementation": { "linkSupport": true },
                "references": {},
                "rename": { "prepareSupport": true },
                "semanticTokens": {
                    "requests": {
                        "range": true,
                        "full": { "delta": true }
                    },
                    "tokenTypes": [
                        "namespace", "type", "class", "enum", "interface", "struct",
                        "typeParameter", "parameter", "variable", "property", "enumMember",
                        "event", "function", "method", "macro", "keyword", "modifier",
                        "comment", "string", "number", "regexp", "operator", "decorator"
                    ],
                    "tokenModifiers": [
                        "declaration", "definition", "readonly", "static", "deprecated",
                        "abstract", "async", "modification", "documentation", "defaultLibrary"
                    ],
                    "formats": ["relative"]
                },
                "inlayHint": {},
                "codeLens": {},
                "documentLink": { "tooltipSupport": true },
                "callHierarchy": {},
                "typeHierarchy": {}
            })),
            workspace: Some(serde_json::json!({
                "workspaceEdit": {
                    "documentChanges": true,
                    "resourceOperations": ["create", "rename", "delete"]
                },
                "workspaceFolders": true,
                "symbol": {}
            })),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitializeParams {
    #[serde(rename = "processId")]
    pub process_id: Option<u32>,
    #[serde(rename = "rootUri")]
    pub root_uri: Option<String>,
    #[serde(rename = "workspaceFolders", default)]
    pub workspace_folders: Option<Vec<WorkspaceFolder>>,
    pub capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceFoldersChangeEvent {
    #[serde(default)]
    pub added: Vec<WorkspaceFolder>,
    #[serde(default)]
    pub removed: Vec<WorkspaceFolder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidChangeWorkspaceFoldersParams {
    pub event: WorkspaceFoldersChangeEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SymbolKind {
    File = 1,
    Module = 2,
    Namespace = 3,
    Package = 4,
    Class = 5,
    Method = 6,
    Property = 7,
    Field = 8,
    Constructor = 9,
    Enum = 10,
    Interface = 11,
    Function = 12,
    Variable = 13,
    Constant = 14,
    String = 15,
    Number = 16,
    Boolean = 17,
    Array = 18,
    Object = 19,
    Key = 20,
    Null = 21,
    EnumMember = 22,
    Struct = 23,
    Event = 24,
    Operator = 25,
    TypeParameter = 26,
}

impl<'de> Deserialize<'de> for SymbolKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct KindVisitor;
        impl<'de> serde::de::Visitor<'de> for KindVisitor {
            type Value = SymbolKind;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an integer symbol kind (1-26) or kind name")
            }

            fn visit_u64<E>(self, value: u64) -> Result<SymbolKind, E>
            where
                E: serde::de::Error,
            {
                match value {
                    1 => Ok(SymbolKind::File),
                    2 => Ok(SymbolKind::Module),
                    3 => Ok(SymbolKind::Namespace),
                    4 => Ok(SymbolKind::Package),
                    5 => Ok(SymbolKind::Class),
                    6 => Ok(SymbolKind::Method),
                    7 => Ok(SymbolKind::Property),
                    8 => Ok(SymbolKind::Field),
                    9 => Ok(SymbolKind::Constructor),
                    10 => Ok(SymbolKind::Enum),
                    11 => Ok(SymbolKind::Interface),
                    12 => Ok(SymbolKind::Function),
                    13 => Ok(SymbolKind::Variable),
                    14 => Ok(SymbolKind::Constant),
                    15 => Ok(SymbolKind::String),
                    16 => Ok(SymbolKind::Number),
                    17 => Ok(SymbolKind::Boolean),
                    18 => Ok(SymbolKind::Array),
                    19 => Ok(SymbolKind::Object),
                    20 => Ok(SymbolKind::Key),
                    21 => Ok(SymbolKind::Null),
                    22 => Ok(SymbolKind::EnumMember),
                    23 => Ok(SymbolKind::Struct),
                    24 => Ok(SymbolKind::Event),
                    25 => Ok(SymbolKind::Operator),
                    26 => Ok(SymbolKind::TypeParameter),
                    _ => Ok(SymbolKind::Variable),
                }
            }

            fn visit_i64<E>(self, value: i64) -> Result<SymbolKind, E>
            where
                E: serde::de::Error,
            {
                self.visit_u64(value as u64)
            }

            fn visit_str<E>(self, value: &str) -> Result<SymbolKind, E>
            where
                E: serde::de::Error,
            {
                match value.to_ascii_lowercase().as_str() {
                    "file" => Ok(SymbolKind::File),
                    "module" => Ok(SymbolKind::Module),
                    "namespace" => Ok(SymbolKind::Namespace),
                    "package" => Ok(SymbolKind::Package),
                    "class" => Ok(SymbolKind::Class),
                    "method" => Ok(SymbolKind::Method),
                    "property" => Ok(SymbolKind::Property),
                    "field" => Ok(SymbolKind::Field),
                    "constructor" => Ok(SymbolKind::Constructor),
                    "enum" => Ok(SymbolKind::Enum),
                    "interface" => Ok(SymbolKind::Interface),
                    "function" => Ok(SymbolKind::Function),
                    "variable" => Ok(SymbolKind::Variable),
                    "constant" => Ok(SymbolKind::Constant),
                    "string" => Ok(SymbolKind::String),
                    "number" => Ok(SymbolKind::Number),
                    "boolean" => Ok(SymbolKind::Boolean),
                    "array" => Ok(SymbolKind::Array),
                    "object" => Ok(SymbolKind::Object),
                    "key" => Ok(SymbolKind::Key),
                    "null" => Ok(SymbolKind::Null),
                    "enummember" => Ok(SymbolKind::EnumMember),
                    "struct" => Ok(SymbolKind::Struct),
                    "event" => Ok(SymbolKind::Event),
                    "operator" => Ok(SymbolKind::Operator),
                    "typeparameter" => Ok(SymbolKind::TypeParameter),
                    _ => Ok(SymbolKind::Variable),
                }
            }
        }

        deserializer.deserialize_any(KindVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolInformation {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(default)]
    pub tags: Option<Vec<u32>>,
    #[serde(default)]
    pub deprecated: Option<bool>,
    pub location: LspLocation,
    #[serde(rename = "containerName", default)]
    pub container_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSymbol {
    pub name: String,
    #[serde(default)]
    pub detail: Option<String>,
    pub kind: SymbolKind,
    #[serde(default)]
    pub tags: Option<Vec<u32>>,
    #[serde(default)]
    pub deprecated: Option<bool>,
    pub range: LspRange,
    #[serde(rename = "selectionRange")]
    pub selection_range: LspRange,
    #[serde(default)]
    pub children: Option<Vec<DocumentSymbol>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceSymbolParams {
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSymbolParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattingOptions {
    #[serde(rename = "tabSize")]
    pub tab_size: u32,
    #[serde(rename = "insertSpaces")]
    pub insert_spaces: bool,
    #[serde(default, rename = "trimTrailingWhitespace", skip_serializing_if = "Option::is_none")]
    pub trim_trailing_whitespace: Option<bool>,
    #[serde(default, rename = "insertFinalNewline", skip_serializing_if = "Option::is_none")]
    pub insert_final_newline: Option<bool>,
    #[serde(default, rename = "trimFinalNewlines", skip_serializing_if = "Option::is_none")]
    pub trim_final_newlines: Option<bool>,
}

impl FormattingOptions {
    pub const fn new(tab_size: u32, insert_spaces: bool) -> Self {
        Self {
            tab_size,
            insert_spaces,
            trim_trailing_whitespace: None,
            insert_final_newline: None,
            trim_final_newlines: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFormattingParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub options: FormattingOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspTextEdit {
    pub range: LspRange,
    #[serde(rename = "newText")]
    pub new_text: String,
}

impl LspTextEdit {
    pub fn new(range: LspRange, new_text: impl Into<String>) -> Self {
        Self {
            range,
            new_text: new_text.into(),
        }
    }
}

pub struct CodeActionKind;

impl CodeActionKind {
    pub const EMPTY: &'static str = "";
    pub const QUICKFIX: &'static str = "quickfix";
    pub const REFACTOR: &'static str = "refactor";
    pub const REFACTOR_EXTRACT: &'static str = "refactor.extract";
    pub const REFACTOR_INLINE: &'static str = "refactor.inline";
    pub const REFACTOR_REWRITE: &'static str = "refactor.rewrite";
    pub const SOURCE: &'static str = "source";
    pub const SOURCE_ORGANIZE_IMPORTS: &'static str = "source.organizeImports";
    pub const SOURCE_FIX_ALL: &'static str = "source.fixAll";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeActionTriggerKind {
    #[serde(rename = "invoked")]
    Invoked = 1,
    #[serde(rename = "automatic")]
    Automatic = 2,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodeActionContext {
    pub diagnostics: Vec<LspDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    #[serde(default, rename = "triggerKind", skip_serializing_if = "Option::is_none")]
    pub trigger_kind: Option<CodeActionTriggerKind>,
}

impl CodeActionContext {
    pub fn new(diagnostics: Vec<LspDiagnostic>) -> Self {
        Self {
            diagnostics,
            only: None,
            trigger_kind: Some(CodeActionTriggerKind::Invoked),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeActionParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub range: LspRange,
    pub context: CodeActionContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspCommand {
    pub title: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<serde_json::Value>>,
}

impl LspCommand {
    pub fn new(title: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            command: command.into(),
            arguments: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionalVersionedTextDocumentIdentifier {
    pub uri: String,
    #[serde(default)]
    pub version: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDocumentEdit {
    #[serde(rename = "textDocument")]
    pub text_document: OptionalVersionedTextDocumentIdentifier,
    pub edits: Vec<LspTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DocumentChange {
    Edit(TextDocumentEdit),
    CreateFile { uri: String },
    RenameFile { old_uri: String, new_uri: String },
    DeleteFile { uri: String },
}

impl<'de> Deserialize<'de> for DocumentChange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        let Some(obj) = val.as_object() else {
            return Err(serde::de::Error::custom("expected DocumentChange object"));
        };
        if let Some(kind) = obj.get("kind").and_then(|k| k.as_str()) {
            match kind {
                "create" => {
                    let uri = obj
                        .get("uri")
                        .and_then(|u| u.as_str())
                        .ok_or_else(|| serde::de::Error::custom("CreateFile missing 'uri'"))?;
                    return Ok(DocumentChange::CreateFile {
                        uri: uri.to_string(),
                    });
                }
                "rename" => {
                    let old_uri = obj
                        .get("oldUri")
                        .and_then(|u| u.as_str())
                        .ok_or_else(|| serde::de::Error::custom("RenameFile missing 'oldUri'"))?;
                    let new_uri = obj
                        .get("newUri")
                        .and_then(|u| u.as_str())
                        .ok_or_else(|| serde::de::Error::custom("RenameFile missing 'newUri'"))?;
                    return Ok(DocumentChange::RenameFile {
                        old_uri: old_uri.to_string(),
                        new_uri: new_uri.to_string(),
                    });
                }
                "delete" => {
                    let uri = obj
                        .get("uri")
                        .and_then(|u| u.as_str())
                        .ok_or_else(|| serde::de::Error::custom("DeleteFile missing 'uri'"))?;
                    return Ok(DocumentChange::DeleteFile {
                        uri: uri.to_string(),
                    });
                }
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "Unknown DocumentChange kind '{}'",
                        other
                    )));
                }
            }
        }
        let edit: TextDocumentEdit =
            serde_json::from_value(val).map_err(serde::de::Error::custom)?;
        Ok(DocumentChange::Edit(edit))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<HashMap<String, Vec<LspTextEdit>>>,
    #[serde(rename = "documentChanges", default, skip_serializing_if = "Option::is_none")]
    pub document_changes: Option<Vec<DocumentChange>>,
}

impl WorkspaceEdit {
    pub fn new() -> Self {
        Self {
            changes: None,
            document_changes: None,
        }
    }

    pub fn with_changes(changes: HashMap<String, Vec<LspTextEdit>>) -> Self {
        Self {
            changes: Some(changes),
            document_changes: None,
        }
    }

    pub fn with_document_changes(document_changes: Vec<DocumentChange>) -> Self {
        Self {
            changes: None,
            document_changes: Some(document_changes),
        }
    }

    pub fn insert_edit(&mut self, uri: impl Into<String>, edit: LspTextEdit) {
        let uri = uri.into();
        let map = self.changes.get_or_insert_with(HashMap::new);
        map.entry(uri).or_default().push(edit);
    }

    /// Collect all per-URI text edits from both `changes` and `documentChanges`.
    pub fn all_text_edits(&self) -> HashMap<String, Vec<LspTextEdit>> {
        let mut combined: HashMap<String, Vec<LspTextEdit>> = HashMap::new();
        if let Some(changes) = &self.changes {
            for (uri, edits) in changes {
                combined.entry(uri.clone()).or_default().extend(edits.clone());
            }
        }
        if let Some(doc_changes) = &self.document_changes {
            for dc in doc_changes {
                if let DocumentChange::Edit(tde) = dc {
                    combined
                        .entry(tde.text_document.uri.clone())
                        .or_default()
                        .extend(tde.edits.clone());
                }
            }
        }
        combined
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterLabel {
    Simple(String),
    Offsets([u32; 2]),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterInformation {
    pub label: ParameterLabel,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_markup_content",
        skip_serializing_if = "Option::is_none"
    )]
    pub documentation: Option<MarkupContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureInformation {
    pub label: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_markup_content",
        skip_serializing_if = "Option::is_none"
    )]
    pub documentation: Option<MarkupContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<ParameterInformation>>,
    #[serde(rename = "activeParameter", default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SignatureHelp {
    #[serde(default)]
    pub signatures: Vec<SignatureInformation>,
    #[serde(rename = "activeSignature", default, skip_serializing_if = "Option::is_none")]
    pub active_signature: Option<u32>,
    #[serde(rename = "activeParameter", default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceContext {
    #[serde(rename = "includeDeclaration")]
    pub include_declaration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrepareRenameResult {
    pub range: LspRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl<'de> Deserialize<'de> for PrepareRenameResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        if let Some(obj) = val.as_object() {
            if let Some(range_val) = obj.get("range") {
                let range: LspRange =
                    serde_json::from_value(range_val.clone()).map_err(serde::de::Error::custom)?;
                let placeholder = obj
                    .get("placeholder")
                    .and_then(|p| p.as_str())
                    .map(ToString::to_string);
                return Ok(Self { range, placeholder });
            }
        }
        let range: LspRange = serde_json::from_value(val).map_err(serde::de::Error::custom)?;
        Ok(Self {
            range,
            placeholder: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SemanticTokensLegend {
    #[serde(rename = "tokenTypes", default)]
    pub token_types: Vec<String>,
    #[serde(rename = "tokenModifiers", default)]
    pub token_modifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SemanticTokens {
    #[serde(rename = "resultId", default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    #[serde(default)]
    pub data: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticTokensEdit {
    pub start: u32,
    #[serde(rename = "deleteCount")]
    pub delete_count: u32,
    #[serde(default)]
    pub data: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticTokensDelta {
    #[serde(rename = "resultId", default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    #[serde(default)]
    pub edits: Vec<SemanticTokensEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSemanticToken {
    pub line: u32,
    pub start_char: u32,
    pub length: u32,
    pub token_type: String,
    pub modifiers: Vec<String>,
}

/// Decode LSP 3.17 relative 5-tuples `[deltaLine, deltaStartChar, length, tokenType, tokenModifiers]`
/// into absolute positions using `legend`. Incomplete trailing elements or unknown type indices
/// are safely skipped without panicking.
pub fn decode_semantic_tokens(
    data: &[u32],
    legend: &SemanticTokensLegend,
) -> Vec<DecodedSemanticToken> {
    let mut out = Vec::with_capacity(data.len() / 5);
    let mut cur_line: u32 = 0;
    let mut cur_char: u32 = 0;

    for chunk in data.as_chunks::<5>().0 {
        let delta_line = chunk[0];
        let delta_start = chunk[1];
        let length = chunk[2];
        let type_idx = chunk[3] as usize;
        let mod_bits = chunk[4];

        if delta_line > 0 {
            cur_line = cur_line.saturating_add(delta_line);
            cur_char = delta_start;
        } else {
            cur_char = cur_char.saturating_add(delta_start);
        }

        let Some(token_type) = legend.token_types.get(type_idx) else {
            continue;
        };

        let mut modifiers = Vec::new();
        for (bit, mod_name) in legend.token_modifiers.iter().enumerate() {
            if bit < 32 && (mod_bits & (1u32 << bit)) != 0 {
                modifiers.push(mod_name.clone());
            }
        }

        out.push(DecodedSemanticToken {
            line: cur_line,
            start_char: cur_char,
            length,
            token_type: token_type.clone(),
            modifiers,
        });
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InlayHintKind {
    Type = 1,
    Parameter = 2,
}

impl Serialize for InlayHintKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for InlayHintKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = u8::deserialize(deserializer)?;
        match val {
            1 => Ok(Self::Type),
            2 => Ok(Self::Parameter),
            _ => Err(serde::de::Error::custom(format!(
                "invalid InlayHintKind: {}",
                val
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InlayHintLabelPart {
    pub value: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_markup_content",
        skip_serializing_if = "Option::is_none"
    )]
    pub tooltip: Option<MarkupContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<LspLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<LspCommand>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InlayHintLabel {
    String(String),
    Parts(Vec<InlayHintLabelPart>),
}

impl InlayHintLabel {
    pub fn display_text(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Parts(parts) => parts.iter().map(|p| p.value.as_str()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InlayHint {
    pub position: LspPosition,
    pub label: InlayHintLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InlayHintKind>,
    #[serde(rename = "paddingLeft", default, skip_serializing_if = "Option::is_none")]
    pub padding_left: Option<bool>,
    #[serde(rename = "paddingRight", default, skip_serializing_if = "Option::is_none")]
    pub padding_right: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_markup_content",
        skip_serializing_if = "Option::is_none"
    )]
    pub tooltip: Option<MarkupContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeLens {
    pub range: LspRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<LspCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentLink {
    pub range: LspRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub uri: String,
    pub range: LspRange,
    #[serde(rename = "selectionRange")]
    pub selection_range: LspRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallHierarchyIncomingCall {
    pub from: CallHierarchyItem,
    #[serde(rename = "fromRanges", default)]
    pub from_ranges: Vec<LspRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallHierarchyOutgoingCall {
    pub to: CallHierarchyItem,
    #[serde(rename = "fromRanges", default)]
    pub from_ranges: Vec<LspRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeHierarchyItem {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub uri: String,
    pub range: LspRange,
    #[serde(rename = "selectionRange")]
    pub selection_range: LspRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeAction {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<LspDiagnostic>>,
    #[serde(default, rename = "isPreferred", skip_serializing_if = "Option::is_none")]
    pub is_preferred: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<WorkspaceEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<LspCommand>,
}

impl CodeAction {
    pub fn new(title: impl Into<String>, kind: Option<String>) -> Self {
        Self {
            title: title.into(),
            kind,
            diagnostics: None,
            is_preferred: None,
            edit: None,
            command: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(rename = "textDocumentSync", default)]
    pub text_document_sync: Option<serde_json::Value>,
    #[serde(rename = "completionProvider", default)]
    pub completion_provider: Option<serde_json::Value>,
    #[serde(rename = "hoverProvider", default)]
    pub hover_provider: Option<serde_json::Value>,
    #[serde(rename = "definitionProvider", default)]
    pub definition_provider: Option<serde_json::Value>,
    #[serde(rename = "signatureHelpProvider", default)]
    pub signature_help_provider: Option<serde_json::Value>,
    #[serde(rename = "referencesProvider", default)]
    pub references_provider: Option<serde_json::Value>,
    #[serde(rename = "implementationProvider", default)]
    pub implementation_provider: Option<serde_json::Value>,
    #[serde(rename = "typeDefinitionProvider", default)]
    pub type_definition_provider: Option<serde_json::Value>,
    #[serde(rename = "renameProvider", default)]
    pub rename_provider: Option<serde_json::Value>,
    #[serde(rename = "semanticTokensProvider", default)]
    pub semantic_tokens_provider: Option<serde_json::Value>,
    #[serde(rename = "inlayHintProvider", default)]
    pub inlay_hint_provider: Option<serde_json::Value>,
    #[serde(rename = "codeLensProvider", default)]
    pub code_lens_provider: Option<serde_json::Value>,
    #[serde(rename = "documentLinkProvider", default)]
    pub document_link_provider: Option<serde_json::Value>,
    #[serde(rename = "callHierarchyProvider", default)]
    pub call_hierarchy_provider: Option<serde_json::Value>,
    #[serde(rename = "typeHierarchyProvider", default)]
    pub type_hierarchy_provider: Option<serde_json::Value>,
}

impl ServerCapabilities {
    pub fn semantic_tokens_legend(&self) -> Option<SemanticTokensLegend> {
        let provider = self.semantic_tokens_provider.as_ref()?;
        let legend_val = provider.get("legend")?;
        serde_json::from_value(legend_val.clone()).ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_item_markup_documentation_deserialization() {
        let json_str = r#"{
            "label": "clone",
            "kind": 2,
            "documentation": {
                "kind": "markdown",
                "value": "Returns a copy of the value."
            }
        }"#;

        let item: LspCompletionItem = serde_json::from_str(json_str).expect("deserialize item");
        assert_eq!(item.label, "clone");
        assert_eq!(item.kind, Some(CompletionItemKind::Method));
        assert_eq!(
            item.documentation.as_deref(),
            Some("Returns a copy of the value.")
        );
    }

    #[test]
    fn test_hover_flexible_deserialization() {
        // 1. Plain string contents
        let json_str = r#"{"contents": "Simple hover"}"#;
        let hover: Hover = serde_json::from_str(json_str).expect("deserialize hover string");
        assert_eq!(hover.contents.value, "Simple hover");

        // 2. Marked string array
        let json_arr = r#"{"contents": [{"language": "rust", "value": "fn foo()"}, "doc text"]}"#;
        let hover_arr: Hover = serde_json::from_str(json_arr).expect("deserialize hover arr");
        assert!(hover_arr.contents.value.contains("```rust\nfn foo()\n```"));
        assert!(hover_arr.contents.value.contains("doc text"));
    }

    #[test]
    fn test_location_link_deserialization() {
        let json_str = r#"{
            "targetUri": "file:///path/to/file.rs",
            "targetSelectionRange": {
                "start": { "line": 10, "character": 4 },
                "end": { "line": 10, "character": 8 }
            }
        }"#;

        let loc: LspLocation = serde_json::from_str(json_str).expect("deserialize location link");
        assert_eq!(loc.uri, "file:///path/to/file.rs");
        assert_eq!(loc.range.start.line, 10);
        assert_eq!(loc.range.start.character, 4);
    }

    #[test]
    fn test_workspace_folders_and_symbols_deserialization() {
        let init_json = r#"{
            "rootUri": "file:///workspace",
            "workspaceFolders": [
                { "uri": "file:///workspace/backend", "name": "backend" },
                { "uri": "file:///workspace/frontend", "name": "frontend" }
            ],
            "capabilities": {}
        }"#;

        let init: InitializeParams = serde_json::from_str(init_json).expect("deserialize init");
        assert_eq!(init.workspace_folders.as_ref().map(|v| v.len()), Some(2));
        assert_eq!(init.workspace_folders.as_ref().unwrap()[0].name, "backend");

        let sym_json = r#"{
            "name": "calculate_total",
            "kind": 12,
            "location": {
                "uri": "file:///workspace/backend/src/calc.rs",
                "range": {
                    "start": { "line": 42, "character": 3 },
                    "end": { "line": 50, "character": 4 }
                }
            },
            "containerName": "Calculator"
        }"#;

        let sym: SymbolInformation = serde_json::from_str(sym_json).expect("deserialize symbol");
        assert_eq!(sym.name, "calculate_total");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.container_name.as_deref(), Some("Calculator"));
    }

    #[test]
    fn test_formatting_types_serde() {
        let opts = FormattingOptions {
            tab_size: 4,
            insert_spaces: true,
            trim_trailing_whitespace: Some(true),
            insert_final_newline: Some(true),
            trim_final_newlines: Some(false),
        };
        let json_str = serde_json::to_string(&opts).expect("serialize formatting options");
        assert!(json_str.contains("\"tabSize\":4"));
        assert!(json_str.contains("\"insertSpaces\":true"));
        assert!(json_str.contains("\"trimTrailingWhitespace\":true"));

        let deserialized: FormattingOptions = serde_json::from_str(&json_str).expect("deserialize formatting options");
        assert_eq!(deserialized, opts);

        let edit = LspTextEdit::new(
            LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 5)),
            "hello",
        );
        let edit_json = serde_json::to_string(&edit).expect("serialize edit");
        assert!(edit_json.contains("\"newText\":\"hello\""));
        let edit_deser: LspTextEdit = serde_json::from_str(&edit_json).expect("deserialize edit");
        assert_eq!(edit_deser, edit);

        let params = DocumentFormattingParams {
            text_document: TextDocumentIdentifier {
                uri: "file:///test.rs".to_string(),
            },
            options: opts,
        };
        let params_json = serde_json::to_string(&params).expect("serialize params");
        let params_deser: DocumentFormattingParams = serde_json::from_str(&params_json).expect("deserialize params");
        assert_eq!(params_deser, params);
    }

    #[test]
    fn test_code_action_types_serde() {
        let mut changes = HashMap::new();
        changes.insert(
            "file:///src/main.rs".to_string(),
            vec![LspTextEdit::new(
                LspRange::new(LspPosition::new(1, 0), LspPosition::new(1, 10)),
                "let x = 42;",
            )],
        );
        let ws_edit = WorkspaceEdit::with_changes(changes);

        let action = CodeAction {
            title: "Replace with let binding".to_string(),
            kind: Some(CodeActionKind::QUICKFIX.to_string()),
            diagnostics: Some(vec![LspDiagnostic {
                range: LspRange::new(LspPosition::new(1, 0), LspPosition::new(1, 10)),
                severity: Some(DiagnosticSeverity::Warning),
                code: Some(DiagnosticCode::String("unused_variable".to_string())),
                source: Some("rust-analyzer".to_string()),
                message: "unused variable".to_string(),
            }]),
            is_preferred: Some(true),
            edit: Some(ws_edit),
            command: Some(LspCommand::new("Apply Fix", "rustcode.applyFix")),
        };

        let action_json = serde_json::to_string(&action).expect("serialize code action");
        assert!(action_json.contains("\"title\":\"Replace with let binding\""));
        assert!(action_json.contains("\"isPreferred\":true"));

        let action_deser: CodeAction = serde_json::from_str(&action_json).expect("deserialize code action");
        assert_eq!(action_deser.title, "Replace with let binding");
        assert_eq!(action_deser.kind.as_deref(), Some(CodeActionKind::QUICKFIX));
        assert!(action_deser.edit.is_some());
        assert_eq!(
            action_deser.edit.as_ref().unwrap().changes.as_ref().unwrap().get("file:///src/main.rs").unwrap().len(),
            1
        );
    }

    #[test]
    fn test_phase9_m1_lsp_types_serde_and_semantic_decoding() {
        // SignatureHelp with offset label and string docs
        let sig_json = r#"{
            "signatures": [{
                "label": "fn add(a: i32, b: i32) -> i32",
                "documentation": "Adds two numbers",
                "parameters": [
                    { "label": [7, 13], "documentation": { "kind": "markdown", "value": "First arg" } },
                    { "label": "b: i32" }
                ]
            }],
            "activeSignature": 0,
            "activeParameter": 1
        }"#;
        let sig: SignatureHelp = serde_json::from_str(sig_json).expect("SignatureHelp serde");
        assert_eq!(sig.signatures.len(), 1);
        assert_eq!(sig.active_parameter, Some(1));
        assert_eq!(
            sig.signatures[0].parameters.as_ref().unwrap()[0].label,
            ParameterLabel::Offsets([7, 13])
        );

        // PrepareRenameResult both object and bare LspRange
        let pr_obj: PrepareRenameResult = serde_json::from_str(
            r#"{"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":5}},"placeholder":"foo"}"#,
        )
        .expect("PrepareRename object");
        assert_eq!(pr_obj.placeholder.as_deref(), Some("foo"));
        let pr_range: PrepareRenameResult = serde_json::from_str(
            r#"{"start":{"line":3,"character":0},"end":{"line":3,"character":4}}"#,
        )
        .expect("PrepareRename bare range");
        assert_eq!(pr_range.range.start.line, 3);
        assert!(pr_range.placeholder.is_none());

        // WorkspaceEdit with both changes and documentChanges
        let ws_json = r#"{
            "changes": {
                "file:///a.rs": [{ "range": { "start": {"line":0,"character":0}, "end": {"line":0,"character":3} }, "newText": "bar" }]
            },
            "documentChanges": [
                {
                    "textDocument": { "uri": "file:///b.rs", "version": 1 },
                    "edits": [{ "range": { "start": {"line":2,"character":4}, "end": {"line":2,"character":7} }, "newText": "bar" }]
                },
                { "kind": "create", "uri": "file:///c.rs" }
            ]
        }"#;
        let ws: WorkspaceEdit = serde_json::from_str(ws_json).expect("WorkspaceEdit documentChanges");
        let all = ws.all_text_edits();
        assert_eq!(all.get("file:///a.rs").unwrap().len(), 1);
        assert_eq!(all.get("file:///b.rs").unwrap().len(), 1);

        // Semantic tokens decoding
        let legend = SemanticTokensLegend {
            token_types: vec!["function".into(), "variable".into()],
            token_modifiers: vec!["declaration".into(), "readonly".into()],
        };
        let raw = vec![
            0, 4, 3, 0, 1, // line 0, col 4, len 3, function, declaration
            2, 2, 5, 1, 2, // line 2, col 2, len 5, variable, readonly
            0, 8, 2, 99, 0, // invalid token_type -> safely skipped
            1, 0, // incomplete trailing -> safely ignored
        ];
        let decoded = decode_semantic_tokens(&raw, &legend);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].line, 0);
        assert_eq!(decoded[0].start_char, 4);
        assert_eq!(decoded[0].token_type, "function");
        assert_eq!(decoded[0].modifiers, vec!["declaration".to_string()]);
        assert_eq!(decoded[1].line, 2);
        assert_eq!(decoded[1].start_char, 2);
        assert_eq!(decoded[1].token_type, "variable");
        assert_eq!(decoded[1].modifiers, vec!["readonly".to_string()]);

        // InlayHint label string + parts
        let hint: InlayHint = serde_json::from_str(
            r#"{"position":{"line":1,"character":10},"label":[{"value":": i32"}],"kind":1,"paddingLeft":true}"#,
        )
        .expect("InlayHint parts");
        assert_eq!(hint.label.display_text(), ": i32");
        assert_eq!(hint.kind, Some(InlayHintKind::Type));

        // ServerCapabilities legend extraction & ClientCapabilities
        let caps: ServerCapabilities = serde_json::from_str(
            r#"{"semanticTokensProvider":{"legend":{"tokenTypes":["type"],"tokenModifiers":["static"]}}}"#,
        )
        .expect("ServerCapabilities");
        assert_eq!(caps.semantic_tokens_legend().unwrap().token_types, vec!["type".to_string()]);
        let client_caps = ClientCapabilities::default_phase9();
        assert!(client_caps.text_document.is_some());
    }
}
