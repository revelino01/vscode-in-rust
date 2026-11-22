//! TextMate `.tmLanguage` (XML plist) and `.tmLanguage.json` grammar loader & tokenizer.
//!
//! Uses `plist` for XML plist parsing, `serde_json` for JSON grammars, and `fancy-regex`
//! for Oniguruma-compatible lookahead/lookbehind/backreference pattern matching.

use crate::syntax::scope::{Grammar, GrammarState, SyntaxScope, TextMateStackFrame, TokenSpan};
use fancy_regex::Regex;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

const MAX_STEPS_PER_LINE: usize = 4096;
const MAX_STACK_DEPTH: usize = 64;
const MAX_INCLUDE_DEPTH: usize = 32;

/// Global interner for runtime TextMate language IDs so `Grammar::language_id()`
/// satisfies `&'static str` without duplicate allocations across reloads.
fn intern_static_str(s: &str) -> &'static str {
    static INTERNER: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let map = INTERNER.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock();
    if let Some(&existing) = guard.get(s) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    guard.insert(s.to_string(), leaked);
    leaked
}

/// Map a TextMate dotted scope name (or space-separated scope list) to `SyntaxScope`.
pub fn map_tm_scope_to_syntax_scope(tm_scope: &str) -> SyntaxScope {
    // Check space-separated scopes from most specific (last) to first, or first match
    for part in tm_scope.split_whitespace().rev() {
        if let Some(mapped) = map_single_tm_scope(part) {
            return mapped;
        }
    }
    SyntaxScope::Plain
}

fn map_single_tm_scope(scope: &str) -> Option<SyntaxScope> {
    let s = scope.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("comment.block.documentation") || s.starts_with("comment.line.documentation") {
        return Some(SyntaxScope::CommentDoc);
    }
    if s.starts_with("comment.line") {
        return Some(SyntaxScope::CommentLine);
    }
    if s.starts_with("comment.block") {
        return Some(SyntaxScope::CommentBlock);
    }
    if s.starts_with("comment") {
        return Some(SyntaxScope::Comment);
    }
    if s.starts_with("keyword.control") {
        return Some(SyntaxScope::KeywordControl);
    }
    if s.starts_with("keyword.operator") {
        return Some(SyntaxScope::KeywordOperator);
    }
    if s.starts_with("keyword.other") {
        return Some(SyntaxScope::KeywordOther);
    }
    if s.starts_with("keyword") {
        return Some(SyntaxScope::Keyword);
    }
    if s.starts_with("storage.type") {
        return Some(SyntaxScope::StorageType);
    }
    if s.starts_with("storage") {
        return Some(SyntaxScope::Keyword);
    }
    if s.starts_with("string.quoted") {
        return Some(SyntaxScope::StringQuoted);
    }
    if s.starts_with("string.template") {
        return Some(SyntaxScope::StringTemplate);
    }
    if s.starts_with("string") {
        return Some(SyntaxScope::String);
    }
    if s.starts_with("constant.numeric") {
        return Some(SyntaxScope::ConstantNumeric);
    }
    if s.starts_with("constant.language") || s.starts_with("constant.character.escape") || s.starts_with("constant") {
        return Some(SyntaxScope::ConstantLanguage);
    }
    if s.starts_with("entity.name.function") || s.starts_with("meta.function-call") {
        return Some(SyntaxScope::EntityNameFunction);
    }
    if s.starts_with("entity.name.type")
        || s.starts_with("entity.name.class")
        || s.starts_with("entity.name.struct")
        || s.starts_with("entity.name.enum")
        || s.starts_with("entity.name.trait")
        || s.starts_with("entity.name.interface")
        || s.starts_with("entity.other.inherited-class")
    {
        return Some(SyntaxScope::EntityNameType);
    }
    if s.starts_with("entity.name.tag") {
        return Some(SyntaxScope::Keyword);
    }
    if s.starts_with("entity.other.attribute-name") || s.starts_with("support.type.property-name") {
        return Some(SyntaxScope::Property);
    }
    if s.starts_with("support.function") {
        return Some(SyntaxScope::SupportFunction);
    }
    if s.starts_with("support.type") || s.starts_with("support.class") {
        return Some(SyntaxScope::Type);
    }
    if s.starts_with("support.constant") {
        return Some(SyntaxScope::ConstantLanguage);
    }
    if s.starts_with("variable.parameter") {
        return Some(SyntaxScope::VariableParameter);
    }
    if s.starts_with("variable.other.property") || s.starts_with("variable.other.member") {
        return Some(SyntaxScope::Property);
    }
    if s.starts_with("variable") {
        return Some(SyntaxScope::Variable);
    }
    if s.starts_with("punctuation.bracket")
        || s.starts_with("punctuation.section")
        || s.starts_with("punctuation.definition.block")
    {
        return Some(SyntaxScope::PunctuationBracket);
    }
    if s.starts_with("punctuation.delimiter")
        || s.starts_with("punctuation.separator")
        || s.starts_with("punctuation.terminator")
    {
        return Some(SyntaxScope::PunctuationDelimiter);
    }
    if s.starts_with("punctuation") {
        return Some(SyntaxScope::Punctuation);
    }
    if s.starts_with("markup.heading") {
        return Some(SyntaxScope::MarkdownHeading);
    }
    if s.starts_with("markup.bold") {
        return Some(SyntaxScope::MarkdownBold);
    }
    if s.starts_with("markup.italic") {
        return Some(SyntaxScope::MarkdownItalic);
    }
    if s.starts_with("markup.raw") || s.starts_with("markup.inline.raw") {
        return Some(SyntaxScope::MarkdownCode);
    }
    if s.starts_with("markup.underline.link") {
        return Some(SyntaxScope::MarkdownLink);
    }
    None
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGrammar {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    scope_name: Option<String>,
    #[serde(default)]
    file_types: Option<Vec<String>>,
    #[serde(default)]
    patterns: Vec<RawPattern>,
    #[serde(default)]
    repository: HashMap<String, RawPattern>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPattern {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    content_name: Option<String>,
    #[serde(default, rename = "match")]
    match_pat: Option<String>,
    #[serde(default)]
    begin: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default, rename = "while")]
    while_pat: Option<String>,
    #[serde(default)]
    captures: HashMap<String, RawCapture>,
    #[serde(default)]
    begin_captures: HashMap<String, RawCapture>,
    #[serde(default)]
    end_captures: HashMap<String, RawCapture>,
    #[serde(default)]
    while_captures: HashMap<String, RawCapture>,
    #[serde(default)]
    patterns: Vec<RawPattern>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    repository: HashMap<String, RawPattern>,
    #[serde(default)]
    apply_end_pattern_last: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawCapture {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    patterns: Vec<RawPattern>,
}

#[derive(Debug, Clone)]
struct CompiledRegex {
    regex: Regex,
    anchor_at_pos: bool,
}

impl CompiledRegex {
    fn compile(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let (anchor_at_pos, cleaned) = if let Some(rest) = trimmed.strip_prefix(r"\G") {
            (true, rest.to_string())
        } else {
            (false, trimmed.replace(r"\G", ""))
        };
        let regex = Regex::new(&cleaned).ok()?;
        Some(Self {
            regex,
            anchor_at_pos,
        })
    }

    fn find_at<'t>(&self, line: &'t str, pos: usize) -> Option<fancy_regex::Captures<'t, str>> {
        if pos > line.len() || !line.is_char_boundary(pos) {
            return None;
        }
        let caps = self.regex.captures_from_pos(line, pos).ok().flatten()?;
        if self.anchor_at_pos {
            let m = caps.get(0)?;
            if m.start() != pos {
                return None;
            }
        }
        Some(caps)
    }
}

#[derive(Debug, Clone)]
struct CompiledCapture {
    group_idx: usize,
    name: Option<String>,
    #[allow(dead_code)]
    patterns: Vec<usize>,
}

#[derive(Debug, Clone)]
enum RuleKind {
    Match {
        regex: Option<CompiledRegex>,
        captures: Vec<CompiledCapture>,
    },
    BeginEnd {
        begin: Option<CompiledRegex>,
        end_raw: Option<String>,
        while_raw: Option<String>,
        begin_captures: Vec<CompiledCapture>,
        end_captures: Vec<CompiledCapture>,
        while_captures: Vec<CompiledCapture>,
        patterns: Vec<usize>,
        apply_end_last: bool,
    },
    Include {
        target: String,
    },
    Group {
        patterns: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
struct CompiledRule {
    name: Option<String>,
    content_name: Option<String>,
    kind: RuleKind,
    local_repo: HashMap<String, usize>,
}

/// Compiled TextMate grammar supporting `.tmLanguage` (XML plist) and `.tmLanguage.json`.
pub struct TextMateGrammar {
    lang_id: &'static str,
    lang_id_owned: String,
    numeric_id: u8,
    scope_name: String,
    display_name: String,
    extensions: Vec<String>,
    rules: Vec<CompiledRule>,
    root_patterns: Vec<usize>,
    repository: HashMap<String, usize>,
    regex_cache: Mutex<HashMap<String, Option<CompiledRegex>>>,
}

impl std::fmt::Debug for TextMateGrammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextMateGrammar")
            .field("lang_id", &self.lang_id_owned)
            .field("scope_name", &self.scope_name)
            .field("rules_len", &self.rules.len())
            .finish()
    }
}

impl TextMateGrammar {
    /// Parse a `.tmLanguage.json` string.
    pub fn from_json_str(
        json_str: &str,
        language_id: &str,
        extensions: &[&str],
    ) -> Result<Self, String> {
        let raw: RawGrammar =
            serde_json::from_str(json_str).map_err(|e| format!("Invalid TextMate JSON: {e}"))?;
        Self::compile_raw(raw, language_id, extensions)
    }

    /// Parse an XML plist `.tmLanguage` string or bytes.
    pub fn from_plist_bytes(
        bytes: &[u8],
        language_id: &str,
        extensions: &[&str],
    ) -> Result<Self, String> {
        let value =
            plist::from_bytes::<plist::Value>(bytes).map_err(|e| format!("Invalid plist: {e}"))?;
        let json_val = plist_value_to_json(&value);
        let raw: RawGrammar = serde_json::from_value(json_val)
            .map_err(|e| format!("Invalid TextMate plist schema: {e}"))?;
        Self::compile_raw(raw, language_id, extensions)
    }

    /// Auto-detect XML plist vs JSON from content string and load grammar.
    pub fn from_str_auto(
        content: &str,
        language_id: &str,
        extensions: &[&str],
    ) -> Result<Self, String> {
        let trimmed = content.trim_start();
        if trimmed.starts_with('{') {
            Self::from_json_str(content, language_id, extensions)
        } else {
            Self::from_plist_bytes(content.as_bytes(), language_id, extensions)
        }
    }

    /// Load a `.tmLanguage` or `.tmLanguage.json` file from disk.
    pub fn from_file(
        path: &Path,
        language_id: &str,
        extensions: &[&str],
    ) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Failed to read grammar {}: {e}", path.display()))?;
        let ext_lower = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if ext_lower == "json" {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| format!("Non-UTF-8 JSON grammar {}: {e}", path.display()))?;
            Self::from_json_str(text, language_id, extensions)
        } else {
            let trimmed = bytes.iter().position(|b| !b.is_ascii_whitespace());
            if trimmed.is_some_and(|i| bytes[i] == b'{') {
                let text = std::str::from_utf8(&bytes)
                    .map_err(|e| format!("Non-UTF-8 JSON grammar {}: {e}", path.display()))?;
                Self::from_json_str(text, language_id, extensions)
            } else {
                Self::from_plist_bytes(&bytes, language_id, extensions)
            }
        }
    }

    pub fn scope_name(&self) -> &str {
        &self.scope_name
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn language_id_str(&self) -> &str {
        &self.lang_id_owned
    }

    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    fn compile_raw(
        raw: RawGrammar,
        language_id: &str,
        extensions: &[&str],
    ) -> Result<Self, String> {
        let scope_name = raw
            .scope_name
            .clone()
            .unwrap_or_else(|| format!("source.{language_id}"));
        let display_name = raw.name.clone().unwrap_or_else(|| language_id.to_string());
        let mut exts: Vec<String> = extensions
            .iter()
            .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if exts.is_empty() {
            if let Some(ft) = &raw.file_types {
                exts = ft
                    .iter()
                    .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        let mut rules = Vec::new();
        let mut repository = HashMap::new();
        for (key, pat) in &raw.repository {
            let id = compile_pattern(pat, &mut rules);
            repository.insert(key.clone(), id);
        }
        let root_patterns: Vec<usize> = raw
            .patterns
            .iter()
            .map(|p| compile_pattern(p, &mut rules))
            .collect();

        Ok(Self {
            lang_id: intern_static_str(language_id),
            lang_id_owned: language_id.to_string(),
            numeric_id: 200,
            scope_name,
            display_name,
            extensions: exts,
            rules,
            root_patterns,
            repository,
            regex_cache: Mutex::new(HashMap::new()),
        })
    }

    fn get_or_compile_regex(&self, pat: &str) -> Option<CompiledRegex> {
        let mut cache = self.regex_cache.lock();
        if let Some(cached) = cache.get(pat) {
            return cached.clone();
        }
        let compiled = CompiledRegex::compile(pat);
        if cache.len() < 1024 {
            cache.insert(pat.to_string(), compiled.clone());
        }
        compiled
    }

    fn collect_active_rule_ids(
        &self,
        list: &[usize],
        local_repos: &[&HashMap<String, usize>],
        visited: &mut HashSet<usize>,
        depth: usize,
        out: &mut Vec<usize>,
    ) {
        if depth >= MAX_INCLUDE_DEPTH {
            return;
        }
        for &id in list {
            let Some(rule) = self.rules.get(id) else {
                continue;
            };
            match &rule.kind {
                RuleKind::Include { target } => {
                    if target == "$self" || target == "$base" {
                        if visited.insert(usize::MAX) {
                            self.collect_active_rule_ids(
                                &self.root_patterns,
                                local_repos,
                                visited,
                                depth + 1,
                                out,
                            );
                        }
                    } else if let Some(repo_key) = target.strip_prefix('#') {
                        let resolved = local_repos
                            .iter()
                            .rev()
                            .find_map(|r| r.get(repo_key).copied())
                            .or_else(|| self.repository.get(repo_key).copied());
                        if let Some(target_id) = resolved {
                            if visited.insert(id) {
                                self.collect_active_rule_ids(
                                    &[target_id],
                                    local_repos,
                                    visited,
                                    depth + 1,
                                    out,
                                );
                            }
                        }
                    }
                }
                RuleKind::Group { patterns } => {
                    if visited.insert(id) {
                        let mut next_repos = local_repos.to_vec();
                        if !rule.local_repo.is_empty() {
                            next_repos.push(&rule.local_repo);
                        }
                        self.collect_active_rule_ids(
                            patterns,
                            &next_repos,
                            visited,
                            depth + 1,
                            out,
                        );
                    }
                }
                RuleKind::Match { .. } | RuleKind::BeginEnd { .. } => {
                    if visited.insert(id) {
                        out.push(id);
                    }
                }
            }
        }
    }

    fn active_patterns_for_stack(&self, stack: &[TextMateStackFrame]) -> Vec<usize> {
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        if let Some(top) = stack.last() {
            if let Some(rule) = self.rules.get(top.rule_id) {
                if let RuleKind::BeginEnd { patterns, .. } = &rule.kind {
                    let mut repos = Vec::new();
                    for f in stack {
                        if let Some(r) = self.rules.get(f.rule_id) {
                            if !r.local_repo.is_empty() {
                                repos.push(&r.local_repo);
                            }
                        }
                    }
                    self.collect_active_rule_ids(patterns, &repos, &mut visited, 0, &mut out);
                }
            }
        } else {
            self.collect_active_rule_ids(&self.root_patterns, &[], &mut visited, 0, &mut out);
        }
        out
    }

    fn emit_captures<'t>(
        &self,
        line: &'t str,
        caps: &fancy_regex::Captures<'t, str>,
        capture_defs: &[CompiledCapture],
        fallback_scope: Option<&str>,
        tokens: &mut Vec<TokenSpan>,
    ) {
        let Some(whole) = caps.get(0) else {
            return;
        };
        let w_start = whole.start();
        let w_end = whole.end();
        if w_start >= w_end || !line.is_char_boundary(w_start) || !line.is_char_boundary(w_end) {
            return;
        }

        // Collect specific sub-captures (group > 0)
        let mut sub_spans: Vec<(usize, usize, String)> = Vec::new();
        let mut group0_scope: Option<String> = None;
        for cd in capture_defs {
            if cd.group_idx == 0 {
                if let Some(name) = &cd.name {
                    group0_scope = Some(name.clone());
                }
                continue;
            }
            if let Some(m) = caps.get(cd.group_idx) {
                if let Some(name) = &cd.name {
                    let s = m.start();
                    let e = m.end();
                    if s < e && line.is_char_boundary(s) && line.is_char_boundary(e) {
                        sub_spans.push((s, e, name.clone()));
                    }
                }
            }
        }

        let outer_scope = group0_scope.or_else(|| fallback_scope.map(ToString::to_string));
        if sub_spans.is_empty() {
            if let Some(scope_str) = outer_scope {
                let mapped = map_tm_scope_to_syntax_scope(&scope_str);
                tokens.push(TokenSpan::with_tm_scope(
                    w_start,
                    w_end,
                    mapped,
                    scope_str,
                    self.numeric_id,
                ));
            }
            return;
        }

        sub_spans.sort_by_key(|(s, e, _)| (*s, *e));
        let mut cur = w_start;
        for (s, e, sc) in sub_spans {
            if s < cur {
                continue;
            }
            if s > cur {
                if let Some(outer) = &outer_scope {
                    let mapped = map_tm_scope_to_syntax_scope(outer);
                    tokens.push(TokenSpan::with_tm_scope(
                        cur,
                        s,
                        mapped,
                        outer.clone(),
                        self.numeric_id,
                    ));
                }
            }
            let mapped = map_tm_scope_to_syntax_scope(&sc);
            tokens.push(TokenSpan::with_tm_scope(
                s,
                e,
                mapped,
                sc,
                self.numeric_id,
            ));
            cur = e;
        }
        if cur < w_end {
            if let Some(outer) = &outer_scope {
                let mapped = map_tm_scope_to_syntax_scope(outer);
                tokens.push(TokenSpan::with_tm_scope(
                    cur,
                    w_end,
                    mapped,
                    outer.clone(),
                    self.numeric_id,
                ));
            }
        }
    }
}

impl Grammar for TextMateGrammar {
    fn language_id(&self) -> &'static str {
        self.lang_id
    }

    fn numeric_id(&self) -> u8 {
        self.numeric_id
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn dynamic_file_extensions(&self) -> Vec<String> {
        self.extensions.clone()
    }

    fn tokenize_line(
        &self,
        line: &str,
        state: Option<&GrammarState>,
    ) -> (Vec<TokenSpan>, Option<GrammarState>) {
        let clean_line = line.trim_end_matches('\r');
        let mut stack: Vec<TextMateStackFrame> = match state {
            Some(GrammarState::TextMate(frames)) => frames.clone(),
            _ => Vec::new(),
        };

        if clean_line.is_empty() {
            // Check while patterns on empty line: if while fails, pop
            let mut valid_len = stack.len();
            for (idx, frame) in stack.iter().enumerate() {
                if let Some(while_pat) = &frame.while_pattern {
                    let matched = self
                        .get_or_compile_regex(while_pat)
                        .and_then(|re| re.find_at(clean_line, 0))
                        .is_some();
                    if !matched {
                        valid_len = idx;
                        break;
                    }
                }
            }
            stack.truncate(valid_len);
            let next_state = if stack.is_empty() {
                None
            } else {
                Some(GrammarState::TextMate(stack))
            };
            return (Vec::new(), next_state);
        }

        let mut tokens: Vec<TokenSpan> = Vec::new();
        let mut pos = 0usize;

        // 1. Validate active `while` continuation rules at start of line
        let mut keep_depth = stack.len();
        for (idx, frame) in stack.iter().enumerate() {
            if let Some(while_pat) = &frame.while_pattern {
                if let Some(caps) = self
                    .get_or_compile_regex(while_pat)
                    .and_then(|re| re.find_at(clean_line, pos))
                {
                    let m = caps.get(0).unwrap();
                    let w_caps = self
                        .rules
                        .get(frame.rule_id)
                        .and_then(|r| match &r.kind {
                            RuleKind::BeginEnd { while_captures, .. } => Some(while_captures.as_slice()),
                            _ => None,
                        })
                        .unwrap_or(&[]);
                    if m.start() > pos {
                        if let Some(cs) = frame.content_scope.as_deref().or_else(|| {
                            self.rules.get(frame.rule_id).and_then(|r| r.name.as_deref())
                        }) {
                            tokens.push(TokenSpan::with_tm_scope(
                                pos,
                                m.start(),
                                map_tm_scope_to_syntax_scope(cs),
                                cs,
                                self.numeric_id,
                            ));
                        }
                    }
                    self.emit_captures(
                        clean_line,
                        &caps,
                        w_caps,
                        frame.content_scope.as_deref(),
                        &mut tokens,
                    );
                    if m.end() > pos {
                        pos = m.end();
                    }
                } else {
                    keep_depth = idx;
                    break;
                }
            }
        }
        stack.truncate(keep_depth);

        // 2. Main scanning loop across `clean_line`
        let mut steps = 0usize;
        while pos < clean_line.len() && steps < MAX_STEPS_PER_LINE {
            steps += 1;

            let mut best_match: Option<(usize, usize, MatchAction)> = None;

            // Check top frame's `end` pattern
            let mut end_first = false;
            if let Some(top) = stack.last() {
                if let Some(rule) = self.rules.get(top.rule_id) {
                    if let RuleKind::BeginEnd { apply_end_last, .. } = &rule.kind {
                        end_first = !apply_end_last;
                    }
                }
                if let Some(end_pat) = &top.end_pattern {
                    if let Some(caps) = self
                        .get_or_compile_regex(end_pat)
                        .and_then(|re| re.find_at(clean_line, pos))
                    {
                        if let Some(m) = caps.get(0) {
                            best_match = Some((m.start(), m.end(), MatchAction::EndFrame));
                        }
                    }
                }
            }

            // Check child / root rules
            let candidates = self.active_patterns_for_stack(&stack);
            for rule_id in candidates {
                let Some(rule) = self.rules.get(rule_id) else {
                    continue;
                };
                let regex_opt = match &rule.kind {
                    RuleKind::Match { regex, .. } => regex.as_ref(),
                    RuleKind::BeginEnd { begin, .. } => begin.as_ref(),
                    _ => None,
                };
                let Some(re) = regex_opt else {
                    continue;
                };
                if let Some(caps) = re.find_at(clean_line, pos) {
                    if let Some(m) = caps.get(0) {
                        let m_start = m.start();
                        let m_end = m.end();
                        let take = match &best_match {
                            None => true,
                            Some((best_s, _, MatchAction::EndFrame)) => {
                                if end_first {
                                    m_start < *best_s
                                } else {
                                    m_start <= *best_s
                                }
                            }
                            Some((best_s, _, _)) => m_start < *best_s,
                        };
                        if take {
                            best_match = Some((m_start, m_end, MatchAction::Rule(rule_id)));
                        }
                    }
                }
            }

            let Some((m_start, m_end, action)) = best_match else {
                // No more matches on this line: emit remaining container scope if inside a rule
                if let Some(scope_str) = active_container_scope(&stack, &self.rules) {
                    tokens.push(TokenSpan::with_tm_scope(
                        pos,
                        clean_line.len(),
                        map_tm_scope_to_syntax_scope(&scope_str),
                        scope_str,
                        self.numeric_id,
                    ));
                }
                break;
            };

            // Emit container scope for text between `pos` and `m_start`
            if m_start > pos {
                if let Some(scope_str) = active_container_scope(&stack, &self.rules) {
                    tokens.push(TokenSpan::with_tm_scope(
                        pos,
                        m_start,
                        map_tm_scope_to_syntax_scope(&scope_str),
                        scope_str,
                        self.numeric_id,
                    ));
                }
            }

            match action {
                MatchAction::EndFrame => {
                    if let Some(frame) = stack.pop() {
                        if let Some(end_pat) = &frame.end_pattern {
                            if let Some(caps) = self
                                .get_or_compile_regex(end_pat)
                                .and_then(|re| re.find_at(clean_line, m_start))
                            {
                                let rule = &self.rules[frame.rule_id];
                                let end_caps = match &rule.kind {
                                    RuleKind::BeginEnd { end_captures, .. } => {
                                        end_captures.as_slice()
                                    }
                                    _ => &[],
                                };
                                let fallback = rule.name.as_deref();
                                self.emit_captures(clean_line, &caps, end_caps, fallback, &mut tokens);
                            }
                        }
                    }
                    if m_end > pos {
                        pos = m_end;
                    } else {
                        pos = next_utf8_step(clean_line, pos);
                    }
                }
                MatchAction::Rule(rule_id) => {
                    let rule = &self.rules[rule_id];
                    match &rule.kind {
                        RuleKind::Match { regex: Some(re), captures } => {
                            if let Some(caps) = re.find_at(clean_line, m_start) {
                                let fallback = rule.name.as_deref().or_else(|| {
                                    active_container_scope_ref(&stack, &self.rules)
                                });
                                self.emit_captures(clean_line, &caps, captures, fallback, &mut tokens);
                            }
                            if m_end > pos {
                                pos = m_end;
                            } else {
                                pos = next_utf8_step(clean_line, pos);
                            }
                        }
                        RuleKind::BeginEnd {
                            begin: Some(re),
                            end_raw,
                            while_raw,
                            begin_captures,
                            ..
                        } => {
                            let resolved_end = if let Some(caps) = re.find_at(clean_line, m_start) {
                                let fallback = rule.name.as_deref();
                                self.emit_captures(
                                    clean_line,
                                    &caps,
                                    begin_captures,
                                    fallback,
                                    &mut tokens,
                                );
                                end_raw.as_ref().map(|e| substitute_backrefs(e, &caps))
                            } else {
                                end_raw.clone()
                            };
                            let resolved_while = if let Some(caps) = re.find_at(clean_line, m_start) {
                                while_raw.as_ref().map(|w| substitute_backrefs(w, &caps))
                            } else {
                                while_raw.clone()
                            };
                            let content_scope = rule
                                .content_name
                                .clone()
                                .or_else(|| rule.name.clone());
                            if stack.len() < MAX_STACK_DEPTH {
                                stack.push(TextMateStackFrame {
                                    rule_id,
                                    end_pattern: resolved_end,
                                    while_pattern: resolved_while,
                                    content_scope,
                                });
                            }
                            if m_end > pos {
                                pos = m_end;
                            } else {
                                pos = next_utf8_step(clean_line, pos);
                            }
                        }
                        _ => {
                            pos = next_utf8_step(clean_line, pos);
                        }
                    }
                }
            }
        }

        // Normalize token spans: sort, remove zero-width/invalid, deduplicate overlaps
        normalize_token_spans(clean_line, &mut tokens);

        let next_state = if stack.is_empty() {
            None
        } else {
            Some(GrammarState::TextMate(stack))
        };
        (tokens, next_state)
    }
}

enum MatchAction {
    EndFrame,
    Rule(usize),
}

fn next_utf8_step(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let ch_len = s[pos..].chars().next().map_or(1, |c| c.len_utf8());
    (pos + ch_len).min(s.len())
}

fn active_container_scope(
    stack: &[TextMateStackFrame],
    rules: &[CompiledRule],
) -> Option<String> {
    active_container_scope_ref(stack, rules).map(ToString::to_string)
}

fn active_container_scope_ref<'a>(
    stack: &'a [TextMateStackFrame],
    rules: &'a [CompiledRule],
) -> Option<&'a str> {
    for frame in stack.iter().rev() {
        if let Some(cs) = &frame.content_scope {
            return Some(cs.as_str());
        }
        if let Some(r) = rules.get(frame.rule_id) {
            if let Some(n) = &r.name {
                return Some(n.as_str());
            }
        }
    }
    None
}

fn substitute_backrefs<'t>(pattern: &str, caps: &fancy_regex::Captures<'t, str>) -> String {
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let digit = (chars[i + 1] as u8 - b'0') as usize;
            if digit > 0 {
                if let Some(m) = caps.get(digit) {
                    out.push_str(&fancy_regex::escape(m.as_str()));
                    i += 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn normalize_token_spans(line: &str, tokens: &mut Vec<TokenSpan>) {
    tokens.retain(|t| {
        t.start < t.end
            && t.end <= line.len()
            && line.is_char_boundary(t.start)
            && line.is_char_boundary(t.end)
    });
    tokens.sort_by_key(|t| (t.start, t.end));
    let mut non_overlapping: Vec<TokenSpan> = Vec::with_capacity(tokens.len());
    let mut cursor = 0usize;
    for mut t in tokens.drain(..) {
        if t.end <= cursor {
            continue;
        }
        if t.start < cursor {
            t.start = cursor;
        }
        if t.start < t.end && line.is_char_boundary(t.start) {
            cursor = t.end;
            non_overlapping.push(t);
        }
    }
    *tokens = non_overlapping;
}

fn compile_captures(
    raw_caps: &HashMap<String, RawCapture>,
    rules: &mut Vec<CompiledRule>,
) -> Vec<CompiledCapture> {
    let mut out = Vec::new();
    for (k, v) in raw_caps {
        if let Ok(idx) = k.parse::<usize>() {
            let sub_pats = v
                .patterns
                .iter()
                .map(|p| compile_pattern(p, rules))
                .collect();
            out.push(CompiledCapture {
                group_idx: idx,
                name: v.name.clone(),
                patterns: sub_pats,
            });
        }
    }
    out.sort_by_key(|c| c.group_idx);
    out
}

fn compile_pattern(pat: &RawPattern, rules: &mut Vec<CompiledRule>) -> usize {
    let mut local_repo = HashMap::new();
    for (k, sub) in &pat.repository {
        let sid = compile_pattern(sub, rules);
        local_repo.insert(k.clone(), sid);
    }

    let kind = if let Some(inc) = &pat.include {
        RuleKind::Include {
            target: inc.clone(),
        }
    } else if pat.begin.is_some() {
        let begin = pat.begin.as_deref().and_then(CompiledRegex::compile);
        let begin_captures = compile_captures(&pat.begin_captures, rules);
        let mut end_captures = compile_captures(&pat.end_captures, rules);
        if end_captures.is_empty() && !pat.captures.is_empty() {
            end_captures = compile_captures(&pat.captures, rules);
        }
        let while_captures = compile_captures(&pat.while_captures, rules);
        let patterns = pat
            .patterns
            .iter()
            .map(|p| compile_pattern(p, rules))
            .collect();
        let apply_end_last = pat
            .apply_end_pattern_last
            .as_ref()
            .is_some_and(|v| v == &serde_json::Value::Bool(true) || v == &serde_json::json!(1));
        RuleKind::BeginEnd {
            begin,
            end_raw: pat.end.clone(),
            while_raw: pat.while_pat.clone(),
            begin_captures,
            end_captures,
            while_captures,
            patterns,
            apply_end_last,
        }
    } else if pat.match_pat.is_some() {
        let regex = pat.match_pat.as_deref().and_then(CompiledRegex::compile);
        let captures = compile_captures(&pat.captures, rules);
        RuleKind::Match { regex, captures }
    } else {
        let patterns = pat
            .patterns
            .iter()
            .map(|p| compile_pattern(p, rules))
            .collect();
        RuleKind::Group { patterns }
    };

    let id = rules.len();
    rules.push(CompiledRule {
        name: pat.name.clone(),
        content_name: pat.content_name.clone(),
        kind,
        local_repo,
    });
    id
}

fn plist_value_to_json(val: &plist::Value) -> serde_json::Value {
    match val {
        plist::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(plist_value_to_json).collect())
        }
        plist::Value::Dictionary(dict) => {
            let mut map = serde_json::Map::new();
            for (k, v) in dict {
                map.insert(k.clone(), plist_value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        plist::Value::Boolean(b) => serde_json::Value::Bool(*b),
        plist::Value::Integer(i) => {
            if let Some(n) = i.as_signed() {
                serde_json::Value::Number(n.into())
            } else if let Some(u) = i.as_unsigned() {
                serde_json::Value::Number(u.into())
            } else {
                serde_json::Value::Null
            }
        }
        plist::Value::Real(r) => serde_json::Number::from_f64(*r)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        plist::Value::String(s) => serde_json::Value::String(s.clone()),
        plist::Value::Data(d) => serde_json::Value::String(String::from_utf8_lossy(d).into_owned()),
        plist::Value::Date(d) => serde_json::Value::String(d.to_xml_format()),
        plist::Value::Uid(u) => serde_json::Value::Number(u.get().into()),
        _ => serde_json::Value::Null,
    }
}

/// Extension grammar contribution entry matching `package.json` `contributes.grammars`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrammarContribution {
    pub language: Option<String>,
    pub scope_name: String,
    pub path: String,
}

/// Load a TextMate grammar from an extension bundle's `contributes.grammars` entry.
pub fn load_textmate_contribution(
    extension_root: &Path,
    contrib: &GrammarContribution,
    extensions: &[&str],
) -> Result<Arc<TextMateGrammar>, String> {
    let raw_rel = contrib.path.trim();
    if raw_rel.is_empty() || raw_rel.contains('\0') {
        return Err("Invalid empty or malformed TextMate grammar path".to_string());
    }
    if raw_rel.starts_with('/') || raw_rel.starts_with('\\') {
        return Err(format!(
            "TextMate grammar path '{}' must be relative to extension root",
            contrib.path
        ));
    }
    let rel_path = Path::new(raw_rel);
    if rel_path.is_absolute() {
        return Err(format!(
            "TextMate grammar path '{}' must not be absolute",
            contrib.path
        ));
    }
    for comp in rel_path.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "TextMate grammar path '{}' escapes extension root",
                    contrib.path
                ));
            }
            std::path::Component::CurDir | std::path::Component::Normal(_) => {}
        }
    }
    // Also guard against Windows-style separators or drive prefixes on Unix hosts.
    if raw_rel.split(['/', '\\']).any(|seg| seg == "..")
        || raw_rel
            .as_bytes()
            .get(1)
            .is_some_and(|&b| b == b':' && raw_rel.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(format!(
            "TextMate grammar path '{}' escapes extension root",
            contrib.path
        ));
    }

    let canon_root = std::fs::canonicalize(extension_root).map_err(|e| {
        format!(
            "Failed to canonicalize extension root {}: {e}",
            extension_root.display()
        )
    })?;
    let joined = extension_root.join(rel_path);
    let canon_target = std::fs::canonicalize(&joined).map_err(|e| {
        format!(
            "Failed to resolve grammar path {}: {e}",
            joined.display()
        )
    })?;
    if !canon_target.starts_with(&canon_root) {
        return Err(format!(
            "TextMate grammar path '{}' escapes extension root '{}'",
            contrib.path,
            extension_root.display()
        ));
    }

    let lang_id = contrib
        .language
        .as_deref()
        .unwrap_or_else(|| {
            contrib
                .scope_name
                .rsplit('.')
                .next()
                .unwrap_or("plaintext")
        });
    let tm = TextMateGrammar::from_file(&canon_target, lang_id, extensions)?;
    Ok(Arc::new(tm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_textmate_json_and_xml_plist_loading_and_tokenization() {
        let json_grammar = r##"{
            "name": "DemoLang",
            "scopeName": "source.demolang",
            "fileTypes": ["demo"],
            "patterns": [
                { "include": "#comments" },
                { "include": "#strings" },
                {
                    "match": "\\b(fn|let|return|if|else)\\b",
                    "name": "keyword.control.demolang"
                },
                {
                    "match": "\\b([a-zA-Z_][a-zA-Z0-9_]*)\\s*(?=\\()",
                    "captures": {
                        "1": { "name": "entity.name.function.demolang" }
                    }
                }
            ],
            "repository": {
                "comments": {
                    "patterns": [
                        {
                            "match": "//.*$",
                            "name": "comment.line.double-slash.demolang"
                        },
                        {
                            "begin": "/\\*",
                            "end": "\\*/",
                            "name": "comment.block.demolang"
                        }
                    ]
                },
                "strings": {
                    "begin": "\"",
                    "end": "\"",
                    "name": "string.quoted.double.demolang",
                    "patterns": [
                        { "match": "\\\\.", "name": "constant.character.escape.demolang" }
                    ]
                }
            }
        }"##;

        let tm = TextMateGrammar::from_json_str(json_grammar, "demolang", &["demo"]).unwrap();
        assert_eq!(tm.language_id(), "demolang");
        assert_eq!(tm.dynamic_file_extensions(), vec!["demo"]);

        // Single-line lookahead + string + comment
        let line0 = "fn compute() { let s = \"hi\\n\"; /* open";
        let (t0, st0) = tm.tokenize_line(line0, None);
        assert!(t0.iter().any(|t| t.scope == SyntaxScope::KeywordControl && &line0[t.start..t.end] == "fn"));
        assert!(t0.iter().any(|t| t.scope == SyntaxScope::EntityNameFunction && &line0[t.start..t.end] == "compute"));
        assert!(t0.iter().any(|t| t.scope == SyntaxScope::StringQuoted));
        assert!(t0.iter().any(|t| t.scope == SyntaxScope::CommentBlock));
        assert!(st0.is_some());

        // Multiline block comment continuation + close
        let line1 = "still in comment";
        let (t1, st1) = tm.tokenize_line(line1, st0.as_ref());
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].scope, SyntaxScope::CommentBlock);
        assert!(st1.is_some());

        let line2 = "end */ return 1;";
        let (t2, st2) = tm.tokenize_line(line2, st1.as_ref());
        assert!(st2.is_none());
        assert_eq!(t2[0].scope, SyntaxScope::CommentBlock);
        assert!(t2.iter().any(|t| t.scope == SyntaxScope::KeywordControl && &line2[t.start..t.end] == "return"));

        // XML plist .tmLanguage format
        let plist_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>PlistLang</string>
    <key>scopeName</key>
    <string>source.plistlang</string>
    <key>patterns</key>
    <array>
        <dict>
            <key>match</key>
            <string>#.*$</string>
            <key>name</key>
            <string>comment.line.number-sign.plistlang</string>
        </dict>
        <dict>
            <key>match</key>
            <string>\b(def|class)\b</string>
            <key>name</key>
            <string>keyword.control.plistlang</string>
        </dict>
    </array>
</dict>
</plist>"#;
        let plist_tm = TextMateGrammar::from_plist_bytes(plist_xml.as_bytes(), "plistlang", &["plg"]).unwrap();
        let (pt, _) = plist_tm.tokenize_line("def greet # comment", None);
        assert!(pt.iter().any(|t| t.scope == SyntaxScope::KeywordControl));
        assert!(pt.iter().any(|t| t.scope == SyntaxScope::CommentLine));
    }

    #[test]
    fn test_textmate_malformed_regex_and_self_include_safe() {
        let bad_json = r#"{
            "scopeName": "source.bad",
            "patterns": [
                { "match": "(unclosed[", "name": "keyword.control" },
                { "include": "$self" },
                { "match": "(?<=\\w)\\w+", "name": "variable.other" }
            ]
        }"#;
        let tm = TextMateGrammar::from_json_str(bad_json, "bad", &[]).unwrap();
        let (tokens, state) = tm.tokenize_line("hello world", None);
        assert!(state.is_none());
        for t in &tokens {
            assert!(t.start < t.end && t.end <= "hello world".len());
        }
    }

    #[test]
    fn test_textmate_while_continuation_and_backreference() {
        let json = r#"{
            "scopeName": "source.heredoc",
            "patterns": [
                {
                    "begin": "<<(\\w+)",
                    "end": "^\\1$",
                    "name": "string.unquoted.heredoc"
                },
                {
                    "begin": "^>",
                    "while": "^>",
                    "name": "comment.block.quote"
                }
            ]
        }"#;
        let tm = TextMateGrammar::from_json_str(json, "heredoc", &[]).unwrap();
        let (_, s0) = tm.tokenize_line("<<EOF", None);
        assert!(s0.is_some());
        let (t1, s1) = tm.tokenize_line("body line", s0.as_ref());
        assert_eq!(t1[0].scope, SyntaxScope::String);
        assert!(s1.is_some());
        let (_, s2) = tm.tokenize_line("EOF", s1.as_ref());
        assert!(s2.is_none());

        // While continuation
        let (_, w0) = tm.tokenize_line("> quoted 1", None);
        assert!(w0.is_some());
        let (wt1, w1) = tm.tokenize_line("> quoted 2", w0.as_ref());
        assert_eq!(wt1[0].scope, SyntaxScope::CommentBlock);
        assert!(w1.is_some());
        let (_, w2) = tm.tokenize_line("normal text", w1.as_ref());
        assert!(w2.is_none());
    }

    #[test]
    fn test_load_textmate_contribution_enforces_extension_root_containment() {
        let base_dir = std::env::temp_dir().join(format!(
            "rustcode_tm_contain_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let ext_root = base_dir.join("ext");
        let syntaxes_dir = ext_root.join("syntaxes");
        std::fs::create_dir_all(&syntaxes_dir).unwrap();

        let grammar_json = r#"{
            "scopeName": "source.demo",
            "patterns": [{ "match": "\\bfn\\b", "name": "keyword.control.demo" }]
        }"#;
        let inside_file = syntaxes_dir.join("demo.tmLanguage.json");
        let outside_file = base_dir.join("outside.tmLanguage.json");
        std::fs::write(&inside_file, grammar_json).unwrap();
        std::fs::write(&outside_file, grammar_json).unwrap();

        // Valid relative path inside extension_root succeeds
        let ok_contrib = GrammarContribution {
            language: Some("demo".to_string()),
            scope_name: "source.demo".to_string(),
            path: "./syntaxes/demo.tmLanguage.json".to_string(),
        };
        assert!(load_textmate_contribution(&ext_root, &ok_contrib, &["demo"]).is_ok());

        // Traversal via `..` or `\` or absolute paths must be rejected
        for bad_path in [
            "../outside.tmLanguage.json",
            "./../outside.tmLanguage.json",
            "syntaxes/../../outside.tmLanguage.json",
            "..\\outside.tmLanguage.json",
            outside_file.to_string_lossy().as_ref(),
            "/etc/passwd",
        ] {
            let bad_contrib = GrammarContribution {
                language: Some("demo".to_string()),
                scope_name: "source.demo".to_string(),
                path: bad_path.to_string(),
            };
            let res = load_textmate_contribution(&ext_root, &bad_contrib, &["demo"]);
            assert!(
                res.is_err(),
                "expected traversal/absolute path '{bad_path}' to be rejected"
            );
        }

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
