//! Semantic syntax scopes, packed TokenMetadata, token spans, and grammar abstractions.

use rustcode_platform::render::Rgba;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxScope {
    Comment,
    CommentLine,
    CommentBlock,
    CommentDoc,
    Keyword,
    KeywordControl,
    KeywordOperator,
    KeywordOther,
    String,
    StringQuoted,
    StringTemplate,
    Number,
    ConstantNumeric,
    ConstantLanguage,
    Type,
    EntityNameType,
    StorageType,
    Function,
    EntityNameFunction,
    SupportFunction,
    Variable,
    VariableParameter,
    Property,
    Operator,
    Punctuation,
    PunctuationBracket,
    PunctuationDelimiter,
    MarkdownHeading,
    MarkdownBold,
    MarkdownItalic,
    MarkdownCode,
    MarkdownLink,
    Plain,
}

impl SyntaxScope {
    /// Return the canonical TextMate scope string representation.
    pub fn as_scope_str(&self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::CommentLine => "comment.line",
            Self::CommentBlock => "comment.block",
            Self::CommentDoc => "comment.block.documentation",
            Self::Keyword => "keyword",
            Self::KeywordControl => "keyword.control",
            Self::KeywordOperator => "keyword.operator",
            Self::KeywordOther => "keyword.other",
            Self::String => "string",
            Self::StringQuoted => "string.quoted",
            Self::StringTemplate => "string.template",
            Self::Number | Self::ConstantNumeric => "constant.numeric",
            Self::ConstantLanguage => "constant.language",
            Self::Type | Self::EntityNameType => "entity.name.type",
            Self::StorageType => "storage.type",
            Self::Function | Self::EntityNameFunction => "entity.name.function",
            Self::SupportFunction => "support.function",
            Self::Variable => "variable",
            Self::VariableParameter => "variable.parameter",
            Self::Property => "variable.other.property",
            Self::Operator => "keyword.operator",
            Self::Punctuation => "punctuation",
            Self::PunctuationBracket => "punctuation.bracket",
            Self::PunctuationDelimiter => "punctuation.delimiter",
            Self::MarkdownHeading => "markup.heading",
            Self::MarkdownBold => "markup.bold",
            Self::MarkdownItalic => "markup.italic",
            Self::MarkdownCode => "markup.raw",
            Self::MarkdownLink => "markup.underline.link",
            Self::Plain => "text",
        }
    }

    /// Convert to a unique 8-bit tag byte for packing into `TokenMetadata`.
    pub fn tag_byte(&self) -> u8 {
        match self {
            Self::Comment => 1,
            Self::CommentLine => 2,
            Self::CommentBlock => 3,
            Self::CommentDoc => 4,
            Self::Keyword => 5,
            Self::KeywordControl => 6,
            Self::KeywordOperator => 7,
            Self::KeywordOther => 8,
            Self::String => 9,
            Self::StringQuoted => 10,
            Self::StringTemplate => 11,
            Self::Number | Self::ConstantNumeric => 12,
            Self::ConstantLanguage => 13,
            Self::Type | Self::EntityNameType => 14,
            Self::StorageType => 15,
            Self::Function | Self::EntityNameFunction => 16,
            Self::SupportFunction => 17,
            Self::Variable => 18,
            Self::VariableParameter => 19,
            Self::Property => 20,
            Self::Operator => 21,
            Self::Punctuation => 22,
            Self::PunctuationBracket => 23,
            Self::PunctuationDelimiter => 24,
            Self::MarkdownHeading => 25,
            Self::MarkdownBold => 26,
            Self::MarkdownItalic => 27,
            Self::MarkdownCode => 28,
            Self::MarkdownLink => 29,
            Self::Plain => 0,
        }
    }

    pub fn from_tag_byte(b: u8) -> Self {
        match b {
            1 => Self::Comment,
            2 => Self::CommentLine,
            3 => Self::CommentBlock,
            4 => Self::CommentDoc,
            5 => Self::Keyword,
            6 => Self::KeywordControl,
            7 => Self::KeywordOperator,
            8 => Self::KeywordOther,
            9 => Self::String,
            10 => Self::StringQuoted,
            11 => Self::StringTemplate,
            12 => Self::Number,
            13 => Self::ConstantLanguage,
            14 => Self::Type,
            15 => Self::StorageType,
            16 => Self::Function,
            17 => Self::SupportFunction,
            18 => Self::Variable,
            19 => Self::VariableParameter,
            20 => Self::Property,
            21 => Self::Operator,
            22 => Self::Punctuation,
            23 => Self::PunctuationBracket,
            24 => Self::PunctuationDelimiter,
            25 => Self::MarkdownHeading,
            26 => Self::MarkdownBold,
            27 => Self::MarkdownItalic,
            28 => Self::MarkdownCode,
            29 => Self::MarkdownLink,
            _ => Self::Plain,
        }
    }
}

/// Token data packed into a 32-bit integer matching VS Code's TokenMetadata:
/// Bits 0-7:   language ID
/// Bits 8-15:  token type / scope tag byte
/// Bits 16-18: font styles (bit 16: bold, bit 17: italic, bit 18: underline)
/// Bits 19-31: reserved
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenMetadata(pub u32);

impl fmt::Debug for TokenMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenMetadata")
            .field("language_id", &self.language_id())
            .field("scope", &self.scope())
            .field("bold", &self.is_bold())
            .field("italic", &self.is_italic())
            .field("underline", &self.is_underline())
            .finish()
    }
}

impl TokenMetadata {
    pub const BOLD_MASK: u32 = 1 << 16;
    pub const ITALIC_MASK: u32 = 1 << 17;
    pub const UNDERLINE_MASK: u32 = 1 << 18;

    pub fn new(lang_id: u8, scope: SyntaxScope, bold: bool, italic: bool, underline: bool) -> Self {
        let mut val = (lang_id as u32) | ((scope.tag_byte() as u32) << 8);
        if bold {
            val |= Self::BOLD_MASK;
        }
        if italic {
            val |= Self::ITALIC_MASK;
        }
        if underline {
            val |= Self::UNDERLINE_MASK;
        }
        Self(val)
    }

    pub fn language_id(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub fn scope(&self) -> SyntaxScope {
        SyntaxScope::from_tag_byte(((self.0 >> 8) & 0xFF) as u8)
    }

    pub fn is_bold(&self) -> bool {
        (self.0 & Self::BOLD_MASK) != 0
    }

    pub fn is_italic(&self) -> bool {
        (self.0 & Self::ITALIC_MASK) != 0
    }

    pub fn is_underline(&self) -> bool {
        (self.0 & Self::UNDERLINE_MASK) != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TokenSpan {
    /// Byte start offset within the line
    pub start: usize,
    /// Byte end offset within the line
    pub end: usize,
    pub scope: SyntaxScope,
    pub metadata: TokenMetadata,
    pub color: Option<Rgba>,
    /// Optional full TextMate scope string (e.g. `"entity.name.function.rust"`).
    pub tm_scope: Option<String>,
}

impl TokenSpan {
    pub fn new(start: usize, end: usize, scope: SyntaxScope, lang_id: u8) -> Self {
        let metadata = TokenMetadata::new(lang_id, scope, false, false, false);
        Self {
            start,
            end,
            scope,
            metadata,
            color: None,
            tm_scope: None,
        }
    }

    pub fn with_style(
        start: usize,
        end: usize,
        scope: SyntaxScope,
        lang_id: u8,
        bold: bool,
        italic: bool,
        underline: bool,
    ) -> Self {
        let metadata = TokenMetadata::new(lang_id, scope, bold, italic, underline);
        Self {
            start,
            end,
            scope,
            metadata,
            color: None,
            tm_scope: None,
        }
    }

    pub fn with_tm_scope(
        start: usize,
        end: usize,
        scope: SyntaxScope,
        tm_scope: impl Into<String>,
        lang_id: u8,
    ) -> Self {
        let metadata = TokenMetadata::new(lang_id, scope, false, false, false);
        Self {
            start,
            end,
            scope,
            metadata,
            color: None,
            tm_scope: Some(tm_scope.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMateStackFrame {
    pub rule_id: usize,
    pub end_pattern: Option<String>,
    pub while_pattern: Option<String>,
    pub content_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarState {
    Normal,
    BlockComment { depth: usize },
    MultiLineString { quote: char },
    TemplateLiteral,
    CodeBlock { lang: String },
    RawString { delimiter: String },
    HtmlInTag,
    HtmlScript,
    HtmlStyle,
    TextMate(Vec<TextMateStackFrame>),
}

/// Tokenizer grammar trait representing an incremental tokenizer for a programming language.
pub trait Grammar: Send + Sync {
    fn language_id(&self) -> &'static str;
    fn numeric_id(&self) -> u8;
    fn file_extensions(&self) -> &'static [&'static str];
    fn dynamic_file_extensions(&self) -> Vec<String> {
        self.file_extensions()
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }
    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>);
}
