use serde::{Deserialize, Serialize};

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
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: Option<DiagnosticSeverity>,
    pub message: String,
    pub source: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "insertText", alias = "insert_text", skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    #[serde(rename = "filterText", alias = "filter_text", skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    #[serde(rename = "sortText", alias = "sort_text", skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
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
    pub contents: MarkupContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<LspRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(rename = "languageId")]
    pub language_id: String,
    pub version: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub text_document: Option<serde_json::Value>,
    #[serde(default)]
    pub workspace: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InitializeParams {
    #[serde(rename = "processId")]
    pub process_id: Option<u32>,
    #[serde(rename = "rootUri")]
    pub root_uri: Option<String>,
    pub capabilities: ClientCapabilities,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
}
