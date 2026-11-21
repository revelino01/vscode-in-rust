//! HTML language tokenizer and grammar implementation.

use crate::syntax::css::CssGrammar;
use crate::syntax::javascript::JavaScriptGrammar;
use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TokenSpan};

pub struct HtmlGrammar;

enum TagEnd {
    Unclosed,
    Closed { self_closing: bool },
}

enum EmbeddedMode {
    Script,
    Style,
}

impl Grammar for HtmlGrammar {
    fn language_id(&self) -> &'static str {
        "html"
    }

    fn numeric_id(&self) -> u8 {
        10
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["html", "htm"]
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let lang_id = self.numeric_id();
        let line = line.trim_end_matches('\r');
        let preserved = match state {
            Some(GrammarState::Normal) | None => None,
            Some(s) => Some(s.clone()),
        };
        if line.bytes().all(|b| b.is_ascii_whitespace()) {
            return (Vec::new(), preserved);
        }

        let mut tokens = Vec::new();
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut embedded: Option<EmbeddedMode> = None;

        match state {
            Some(GrammarState::BlockComment { .. }) => {
                let start = i;
                let mut closed = false;
                while i < len {
                    if i + 2 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' && bytes[i + 2] == b'>'
                    {
                        i += 3;
                        closed = true;
                        break;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
                if !closed {
                    return (tokens, Some(GrammarState::BlockComment { depth: 1 }));
                }
            }
            Some(GrammarState::HtmlInTag) => {
                if let TagEnd::Unclosed = scan_tag_contents(line, &mut i, &mut tokens, lang_id) {
                    return (tokens, Some(GrammarState::HtmlInTag));
                }
            }
            Some(GrammarState::HtmlScript) => {
                embedded = Some(EmbeddedMode::Script);
            }
            Some(GrammarState::HtmlStyle) => {
                embedded = Some(EmbeddedMode::Style);
            }
            _ => {}
        }

        while i < len {
            if let Some(mode) = embedded.take() {
                let needle = match mode {
                    EmbeddedMode::Script => "</script",
                    EmbeddedMode::Style => "</style",
                };
                if let Some(rel_end) = find_closing_tag_ascii(&line[i..], needle) {
                    let end = i + rel_end;
                    if end > i {
                        emit_embedded(&line[i..end], i, &mode, &mut tokens, lang_id);
                    }
                    i = end;
                } else {
                    emit_embedded(&line[i..], i, &mode, &mut tokens, lang_id);
                    let next_state = match mode {
                        EmbeddedMode::Script => GrammarState::HtmlScript,
                        EmbeddedMode::Style => GrammarState::HtmlStyle,
                    };
                    return (tokens, Some(next_state));
                }
            }

            let b = bytes[i];

            if b.is_ascii_whitespace() {
                i += 1;
                continue;
            }

            // HTML comment <!-- ... -->
            if b == b'<' && i + 3 < len && &bytes[i..i + 4] == b"<!--" {
                let start = i;
                i += 4;
                let mut closed = false;
                while i < len {
                    if i + 2 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' && bytes[i + 2] == b'>'
                    {
                        i += 3;
                        closed = true;
                        break;
                    }
                    let ch = line[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                }
                tokens.push(TokenSpan::new(start, i, SyntaxScope::CommentBlock, lang_id));
                if !closed {
                    return (tokens, Some(GrammarState::BlockComment { depth: 1 }));
                }
                continue;
            }

            // DOCTYPE / declaration <!...>
            if b == b'<' && i + 1 < len && bytes[i + 1] == b'!' {
                tokens.push(TokenSpan::new(i, i + 2, SyntaxScope::PunctuationBracket, lang_id));
                i += 2;
                let name_start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                    i += 1;
                }
                if i > name_start {
                    tokens.push(TokenSpan::new(name_start, i, SyntaxScope::Keyword, lang_id));
                }
                if let TagEnd::Unclosed = scan_tag_contents(line, &mut i, &mut tokens, lang_id) {
                    return (tokens, Some(GrammarState::HtmlInTag));
                }
                continue;
            }

            // Closing tag </tag>
            if b == b'<'
                && i + 2 < len
                && bytes[i + 1] == b'/'
                && (bytes[i + 2].is_ascii_alphabetic() || bytes[i + 2] == b'_')
            {
                tokens.push(TokenSpan::new(i, i + 2, SyntaxScope::PunctuationBracket, lang_id));
                i += 2;
                let tag_start = i;
                while i < len
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'-'
                        || bytes[i] == b'_'
                        || bytes[i] == b':')
                {
                    i += 1;
                }
                tokens.push(TokenSpan::new(tag_start, i, SyntaxScope::Keyword, lang_id));
                if let TagEnd::Unclosed = scan_tag_contents(line, &mut i, &mut tokens, lang_id) {
                    return (tokens, Some(GrammarState::HtmlInTag));
                }
                continue;
            }

            // Opening tag <tag ...>
            if b == b'<' && i + 1 < len && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') {
                tokens.push(TokenSpan::new(i, i + 1, SyntaxScope::PunctuationBracket, lang_id));
                i += 1;
                let tag_start = i;
                while i < len
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'-'
                        || bytes[i] == b'_'
                        || bytes[i] == b':')
                {
                    i += 1;
                }
                let tag_name = &line[tag_start..i];
                tokens.push(TokenSpan::new(tag_start, i, SyntaxScope::Keyword, lang_id));

                match scan_tag_contents(line, &mut i, &mut tokens, lang_id) {
                    TagEnd::Unclosed => {
                        return (tokens, Some(GrammarState::HtmlInTag));
                    }
                    TagEnd::Closed { self_closing } => {
                        if !self_closing {
                            if tag_name.eq_ignore_ascii_case("script") {
                                embedded = Some(EmbeddedMode::Script);
                            } else if tag_name.eq_ignore_ascii_case("style") {
                                embedded = Some(EmbeddedMode::Style);
                            }
                        }
                    }
                }
                continue;
            }

            // HTML entity &amp; / &#123; / &#x1F600;
            if b == b'&' {
                let start = i;
                let mut probe = i + 1;
                let mut entity_end = None;
                let limit = (i + 20).min(len);
                if probe < limit && bytes[probe] == b'#' {
                    probe += 1;
                }
                while probe < limit {
                    if bytes[probe] == b';' && probe > start + 1 {
                        entity_end = Some(probe + 1);
                        break;
                    }
                    if !bytes[probe].is_ascii_alphanumeric() {
                        break;
                    }
                    probe += 1;
                }
                if let Some(end) = entity_end {
                    tokens.push(TokenSpan::new(start, end, SyntaxScope::ConstantLanguage, lang_id));
                    i = end;
                    continue;
                }
                tokens.push(TokenSpan::new(start, start + 1, SyntaxScope::Plain, lang_id));
                i += 1;
                continue;
            }

            // Text content (stop before <, &, or whitespace)
            let start = i;
            while i < len {
                let cur = bytes[i];
                if cur == b'<' || cur == b'&' || cur.is_ascii_whitespace() {
                    break;
                }
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
            }
            if i > start {
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Plain, lang_id));
            } else {
                let ch = line[i..].chars().next().unwrap();
                i += ch.len_utf8();
                tokens.push(TokenSpan::new(start, i, SyntaxScope::Plain, lang_id));
            }
        }

        let end_state = match embedded {
            Some(EmbeddedMode::Script) => Some(GrammarState::HtmlScript),
            Some(EmbeddedMode::Style) => Some(GrammarState::HtmlStyle),
            None => None,
        };
        (tokens, end_state)
    }
}

fn scan_tag_contents(
    line: &str,
    i: &mut usize,
    tokens: &mut Vec<TokenSpan>,
    lang_id: u8,
) -> TagEnd {
    let bytes = line.as_bytes();
    let len = bytes.len();

    while *i < len {
        let b = bytes[*i];

        if b.is_ascii_whitespace() {
            *i += 1;
            continue;
        }

        if b == b'/' && *i + 1 < len && bytes[*i + 1] == b'>' {
            tokens.push(TokenSpan::new(
                *i,
                *i + 2,
                SyntaxScope::PunctuationBracket,
                lang_id,
            ));
            *i += 2;
            return TagEnd::Closed { self_closing: true };
        }

        if b == b'>' {
            tokens.push(TokenSpan::new(
                *i,
                *i + 1,
                SyntaxScope::PunctuationBracket,
                lang_id,
            ));
            *i += 1;
            return TagEnd::Closed {
                self_closing: false,
            };
        }

        if b == b'=' {
            tokens.push(TokenSpan::new(*i, *i + 1, SyntaxScope::Operator, lang_id));
            *i += 1;
            continue;
        }

        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = *i;
            *i += 1;
            while *i < len {
                if bytes[*i] == quote {
                    *i += 1;
                    break;
                }
                let ch = line[*i..].chars().next().unwrap();
                *i += ch.len_utf8();
            }
            tokens.push(TokenSpan::new(start, *i, SyntaxScope::String, lang_id));
            continue;
        }

        // Attribute name or unquoted value
        let start = *i;
        while *i < len {
            let cur = bytes[*i];
            if cur.is_ascii_whitespace()
                || matches!(cur, b'=' | b'>' | b'"' | b'\'' | b'<')
                || (cur == b'/' && *i + 1 < len && bytes[*i + 1] == b'>')
            {
                break;
            }
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
        }
        if *i > start {
            tokens.push(TokenSpan::new(start, *i, SyntaxScope::Property, lang_id));
        } else {
            let ch = line[*i..].chars().next().unwrap();
            *i += ch.len_utf8();
            tokens.push(TokenSpan::new(start, *i, SyntaxScope::Plain, lang_id));
        }
    }

    TagEnd::Unclosed
}

fn find_closing_tag_ascii(haystack: &str, needle_lower: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle_lower.as_bytes();
    if h.len() < n.len() {
        return None;
    }
    for pos in 0..=h.len() - n.len() {
        if h[pos..pos + n.len()].eq_ignore_ascii_case(n) {
            let after = pos + n.len();
            if after == h.len() || h[after] == b'>' || h[after].is_ascii_whitespace() {
                return Some(pos);
            }
        }
    }
    None
}

fn emit_embedded(
    sub: &str,
    offset: usize,
    mode: &EmbeddedMode,
    tokens: &mut Vec<TokenSpan>,
    lang_id: u8,
) {
    let (sub_tokens, _) = match mode {
        EmbeddedMode::Script => JavaScriptGrammar.tokenize_line(sub, None),
        EmbeddedMode::Style => CssGrammar.tokenize_line(sub, None),
    };
    for span in sub_tokens {
        tokens.push(TokenSpan::new(
            offset + span.start,
            offset + span.end,
            span.scope,
            lang_id,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_tags_attributes_entities_and_comments() {
        let g = HtmlGrammar;
        let line = "<div class=\"hero\"><!-- note -->Hi &amp; bye</div>";
        let (tokens, state) = g.tokenize_line(line, None);
        assert!(state.is_none());
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Keyword && &line[t.start..t.end] == "div"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::Property && &line[t.start..t.end] == "class"));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::String && &line[t.start..t.end] == "\"hero\""));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::CommentBlock));
        assert!(tokens.iter().any(|t| t.scope == SyntaxScope::ConstantLanguage && &line[t.start..t.end] == "&amp;"));
    }

    #[test]
    fn test_html_multiline_comment_tag_and_script_style() {
        let g = HtmlGrammar;
        let (_, c1) = g.tokenize_line("<!-- open", None);
        assert_eq!(c1, Some(GrammarState::BlockComment { depth: 1 }));
        let (ct2, c2) = g.tokenize_line("mid --> <p>", c1.as_ref());
        assert_eq!(c2, None);
        assert_eq!(ct2[0].scope, SyntaxScope::CommentBlock);

        let (_, s1) = g.tokenize_line("<script>", None);
        assert_eq!(s1, Some(GrammarState::HtmlScript));
        let (st2, s2) = g.tokenize_line("const x = 1;", s1.as_ref());
        assert_eq!(s2, Some(GrammarState::HtmlScript));
        assert!(st2.iter().any(|t| t.scope == SyntaxScope::Keyword));
        let (st3, s3) = g.tokenize_line("</script>", s2.as_ref());
        assert_eq!(s3, None);
        assert!(st3.iter().any(|t| t.scope == SyntaxScope::Keyword));
    }

    #[test]
    fn test_html_whitespace_crlf_utf8() {
        let g = HtmlGrammar;
        assert!(g.tokenize_line("", None).0.is_empty());
        assert!(g.tokenize_line("  \t\r", None).0.is_empty());

        let line = "<p title=\"🚀 café\">Hello 🚀 日本語</p>\r";
        let (tokens, _) = g.tokenize_line(line, None);
        for t in &tokens {
            assert!(t.start < t.end && t.end <= line.len());
            assert!(line.is_char_boundary(t.start) && line.is_char_boundary(t.end));
            assert!(!line[t.start..t.end].ends_with('\r'));
        }
    }
}
