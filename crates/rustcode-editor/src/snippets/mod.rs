//! TextMate / VS Code Snippets parser, expansion engine, session state, and JSON registry.

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Context supplying VS Code built-in snippet variables (`$TM_*`, `$CURRENT_*`, commented delimiters).
#[derive(Debug, Clone, Default)]
pub struct SnippetContext {
    pub tm_filename: String,
    pub tm_filename_base: String,
    pub tm_directory: String,
    pub tm_filepath: String,
    pub tm_line_index: usize,
    pub tm_line_number: usize,
    pub tm_selected_text: String,
    pub tm_current_line: String,
    pub tm_current_word: String,
    pub clipboard: String,
    pub line_comment: String,
    pub block_comment_start: String,
    pub block_comment_end: String,
    pub year: String,
    pub month: String,
    pub day: String,
    pub hour: String,
    pub minute: String,
    pub second: String,
}

fn utc_calendar_now() -> (String, String, String, String, String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Howard Hinnant's civil-from-days algorithm
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    (
        format!("{:04}", year),
        format!("{:02}", m),
        format!("{:02}", d),
        format!("{:02}", hour),
        format!("{:02}", minute),
        format!("{:02}", second),
    )
}

impl SnippetContext {
    pub fn from_editor(
        filepath: Option<&Path>,
        line_idx: usize,
        line_text: &str,
        selected_text: &str,
        current_word: &str,
        clipboard: &str,
    ) -> Self {
        let (year, month, day, hour, minute, second) = utc_calendar_now();
        let tm_filepath = filepath
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tm_filename = filepath
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let tm_filename_base = filepath
            .and_then(|p| p.file_stem())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let tm_directory = filepath
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        Self {
            tm_filename,
            tm_filename_base,
            tm_directory,
            tm_filepath,
            tm_line_index: line_idx,
            tm_line_number: line_idx + 1,
            tm_selected_text: selected_text.to_string(),
            tm_current_line: line_text.to_string(),
            tm_current_word: current_word.to_string(),
            clipboard: clipboard.to_string(),
            line_comment: "//".to_string(),
            block_comment_start: "/*".to_string(),
            block_comment_end: "*/".to_string(),
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    pub fn resolve_variable(&self, name: &str) -> Option<String> {
        let now = if self.year.is_empty() {
            Some(utc_calendar_now())
        } else {
            None
        };
        let year = now.as_ref().map_or(self.year.as_str(), |t| t.0.as_str());
        let month = now.as_ref().map_or(self.month.as_str(), |t| t.1.as_str());
        let day = now.as_ref().map_or(self.day.as_str(), |t| t.2.as_str());
        let hour = now.as_ref().map_or(self.hour.as_str(), |t| t.3.as_str());
        let minute = now.as_ref().map_or(self.minute.as_str(), |t| t.4.as_str());
        let second = now.as_ref().map_or(self.second.as_str(), |t| t.5.as_str());

        match name {
            "TM_FILENAME" => Some(self.tm_filename.clone()),
            "TM_FILENAME_BASE" => Some(self.tm_filename_base.clone()),
            "TM_DIRECTORY" => Some(self.tm_directory.clone()),
            "TM_FILEPATH" => Some(self.tm_filepath.clone()),
            "TM_LINE_INDEX" => Some(self.tm_line_index.to_string()),
            "TM_LINE_NUMBER" => Some(self.tm_line_number.to_string()),
            "TM_SELECTED_TEXT" | "SELECTION" => Some(self.tm_selected_text.clone()),
            "TM_CURRENT_LINE" => Some(self.tm_current_line.clone()),
            "TM_CURRENT_WORD" => Some(self.tm_current_word.clone()),
            "CLIPBOARD" => Some(self.clipboard.clone()),
            "LINE_COMMENT" => Some(self.line_comment.clone()),
            "BLOCK_COMMENT_START" => Some(self.block_comment_start.clone()),
            "BLOCK_COMMENT_END" => Some(self.block_comment_end.clone()),
            "CURRENT_YEAR" => Some(year.to_string()),
            "CURRENT_YEAR_SHORT" => Some(year.chars().rev().take(2).collect::<String>().chars().rev().collect()),
            "CURRENT_MONTH" => Some(month.to_string()),
            "CURRENT_DATE" => Some(day.to_string()),
            "CURRENT_HOUR" => Some(hour.to_string()),
            "CURRENT_MINUTE" => Some(minute.to_string()),
            "CURRENT_SECOND" => Some(second.to_string()),
            "CURRENT_SECONDS_UNIX" => Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
                    .to_string(),
            ),
            "UUID" => Some(Uuid::new_v4().to_string()),
            "RANDOM" => {
                let b = *Uuid::new_v4().as_bytes();
                let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 1_000_000;
                Some(format!("{:06}", n))
            }
            "RANDOM_HEX" => {
                let b = *Uuid::new_v4().as_bytes();
                let n = u32::from_le_bytes([b[0], b[1], b[2], 0]) & 0x00FF_FFFF;
                Some(format!("{:06x}", n))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetTransform {
    pub regex: String,
    pub format: String,
    pub flags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetNode {
    Text(String),
    Tabstop {
        index: u32,
        placeholder: Vec<SnippetNode>,
        transform: Option<SnippetTransform>,
        choices: Vec<String>,
    },
    Variable {
        name: String,
        fallback: Vec<SnippetNode>,
        transform: Option<SnippetTransform>,
    },
}

fn transform_case(s: &str, mode: &str) -> String {
    match mode {
        "upcase" => s.to_uppercase(),
        "downcase" => s.to_lowercase(),
        "capitalize" => {
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }
        "pascalcase" => s
            .split(|c: char| !c.is_alphanumeric())
            .filter(|p| !p.is_empty())
            .map(|p| transform_case(p, "capitalize"))
            .collect(),
        "camelcase" => {
            let mut parts = s
                .split(|c: char| !c.is_alphanumeric())
                .filter(|p| !p.is_empty());
            let mut out = String::new();
            if let Some(first) = parts.next() {
                out.push_str(&first.to_lowercase());
            }
            for p in parts {
                out.push_str(&transform_case(p, "capitalize"));
            }
            out
        }
        _ => s.to_string(),
    }
}

fn format_transform_replacement(caps: &Captures<'_>, fmt: &str) -> String {
    let mut out = String::new();
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1] as char;
            match next {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                _ => out.push(next),
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                if let Some(close_rel) = fmt[i + 2..].find('}') {
                    let inner = &fmt[i + 2..i + 2 + close_rel];
                    if let Some((idx_s, rest)) = inner.split_once(":/") {
                        if let Ok(idx) = idx_s.parse::<usize>() {
                            let val = caps.get(idx).map_or("", |m| m.as_str());
                            out.push_str(&transform_case(val, rest));
                        }
                    } else if let Ok(idx) = inner.parse::<usize>() {
                        out.push_str(caps.get(idx).map_or("", |m| m.as_str()));
                    }
                    i += 3 + close_rel;
                    continue;
                }
            } else {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 {
                    if let Ok(idx) = fmt[i + 1..j].parse::<usize>() {
                        out.push_str(caps.get(idx).map_or("", |m| m.as_str()));
                    }
                    i = j;
                    continue;
                }
            }
        }
        let ch = fmt[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn apply_snippet_transform(input: &str, tr: &SnippetTransform) -> String {
    let case_ins = tr.flags.contains('i');
    let global = tr.flags.contains('g');
    let pat = if case_ins {
        format!("(?i){}", tr.regex)
    } else {
        tr.regex.clone()
    };
    let Ok(re) = Regex::new(&pat) else {
        return input.to_string();
    };
    if global {
        re.replace_all(input, |caps: &Captures<'_>| {
            format_transform_replacement(caps, &tr.format)
        })
        .into_owned()
    } else {
        re.replace(input, |caps: &Captures<'_>| {
            format_transform_replacement(caps, &tr.format)
        })
        .into_owned()
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn read_until_unescaped(&mut self, stop: char) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if ch == stop {
                break;
            }
            if ch == '\\' {
                self.advance_char();
                if let Some(esc) = self.advance_char() {
                    out.push(esc);
                }
            } else {
                out.push(self.advance_char().unwrap());
            }
        }
        out
    }

    fn parse_nodes(&mut self, stop_on_brace: bool) -> Vec<SnippetNode> {
        let mut nodes = Vec::new();
        let mut text_buf = String::new();
        let flush = |buf: &mut String, out: &mut Vec<SnippetNode>| {
            if !buf.is_empty() {
                out.push(SnippetNode::Text(std::mem::take(buf)));
            }
        };

        while let Some(ch) = self.peek() {
            if stop_on_brace && ch == '}' {
                self.advance_char();
                break;
            }
            if ch == '\\' {
                self.advance_char();
                if let Some(esc) = self.advance_char() {
                    text_buf.push(esc);
                }
                continue;
            }
            if ch == '$' {
                let save = self.pos;
                self.advance_char();
                if let Some(node) = self.try_parse_dollar() {
                    flush(&mut text_buf, &mut nodes);
                    nodes.push(node);
                    continue;
                } else {
                    self.pos = save;
                    text_buf.push(self.advance_char().unwrap());
                    continue;
                }
            }
            text_buf.push(self.advance_char().unwrap());
        }
        flush(&mut text_buf, &mut nodes);
        nodes
    }

    fn parse_digits(&mut self) -> Option<u32> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance_char();
            } else {
                break;
            }
        }
        if self.pos == start {
            None
        } else {
            self.src[start..self.pos].parse().ok()
        }
    }

    fn parse_ident(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.advance_char();
            } else {
                break;
            }
        }
        if self.pos == start {
            None
        } else {
            Some(self.src[start..self.pos].to_string())
        }
    }

    fn read_transform_format(&mut self) -> String {
        let mut out = String::new();
        let mut brace_depth = 0usize;
        while let Some(ch) = self.peek() {
            if ch == '/' && brace_depth == 0 {
                break;
            }
            if ch == '\\' {
                self.advance_char();
                if let Some(esc) = self.advance_char() {
                    if esc == '/' {
                        out.push('/');
                    } else {
                        out.push('\\');
                        out.push(esc);
                    }
                }
                continue;
            }
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' && brace_depth > 0 {
                brace_depth -= 1;
            }
            out.push(self.advance_char().unwrap());
        }
        out
    }

    fn parse_transform_after_slash(&mut self) -> Option<SnippetTransform> {
        self.advance_char(); // '/'
        let regex = self.read_until_unescaped('/');
        if self.peek() != Some('/') {
            return None;
        }
        self.advance_char(); // '/'
        let format = self.read_transform_format();
        if self.peek() != Some('/') {
            return None;
        }
        self.advance_char(); // '/'
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c == '}' {
                self.advance_char();
                break;
            }
            flags.push(self.advance_char().unwrap());
        }
        Some(SnippetTransform {
            regex,
            format,
            flags,
        })
    }

    fn try_parse_dollar(&mut self) -> Option<SnippetNode> {
        if let Some(idx) = self.parse_digits() {
            return Some(SnippetNode::Tabstop {
                index: idx,
                placeholder: Vec::new(),
                transform: None,
                choices: Vec::new(),
            });
        }
        if let Some(name) = self.parse_ident() {
            return Some(SnippetNode::Variable {
                name,
                fallback: Vec::new(),
                transform: None,
            });
        }
        if self.peek() == Some('{') {
            self.advance_char();
            if let Some(idx) = self.parse_digits() {
                return match self.peek() {
                    Some('}') => {
                        self.advance_char();
                        Some(SnippetNode::Tabstop {
                            index: idx,
                            placeholder: Vec::new(),
                            transform: None,
                            choices: Vec::new(),
                        })
                    }
                    Some(':') => {
                        self.advance_char();
                        let placeholder = self.parse_nodes(true);
                        Some(SnippetNode::Tabstop {
                            index: idx,
                            placeholder,
                            transform: None,
                            choices: Vec::new(),
                        })
                    }
                    Some('|') => {
                        self.advance_char();
                        let raw = self.read_until_unescaped('|');
                        if self.peek() == Some('|') {
                            self.advance_char();
                        }
                        if self.peek() == Some('}') {
                            self.advance_char();
                        }
                        let choices: Vec<String> = raw
                            .split(',')
                            .map(|s| s.replace("\\,", ",").replace("\\|", "|"))
                            .collect();
                        Some(SnippetNode::Tabstop {
                            index: idx,
                            placeholder: Vec::new(),
                            transform: None,
                            choices,
                        })
                    }
                    Some('/') => {
                        let transform = self.parse_transform_after_slash();
                        Some(SnippetNode::Tabstop {
                            index: idx,
                            placeholder: Vec::new(),
                            transform,
                            choices: Vec::new(),
                        })
                    }
                    _ => None,
                };
            }
            if let Some(name) = self.parse_ident() {
                return match self.peek() {
                    Some('}') => {
                        self.advance_char();
                        Some(SnippetNode::Variable {
                            name,
                            fallback: Vec::new(),
                            transform: None,
                        })
                    }
                    Some(':') => {
                        self.advance_char();
                        let fallback = self.parse_nodes(true);
                        Some(SnippetNode::Variable {
                            name,
                            fallback,
                            transform: None,
                        })
                    }
                    Some('/') => {
                        let transform = self.parse_transform_after_slash();
                        Some(SnippetNode::Variable {
                            name,
                            fallback: Vec::new(),
                            transform,
                        })
                    }
                    _ => None,
                };
            }
        }
        None
    }
}

pub fn parse_snippet(template: &str) -> Vec<SnippetNode> {
    Parser::new(template).parse_nodes(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabstopSite {
    pub start: usize,
    pub end: usize,
    pub is_mirror: bool,
    pub transform: Option<SnippetTransform>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabstopGroup {
    pub index: u32,
    pub sites: Vec<TabstopSite>,
    pub choices: Vec<String>,
    pub current_choice: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedSnippet {
    pub text: String,
    pub tabstops: Vec<TabstopGroup>,
}

/// Interactive session managing tabstops, linked mirrors, and choice lists after snippet insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetSession {
    pub tabstops: Vec<TabstopGroup>,
    pub active_group: usize,
    pub finished: bool,
}

impl SnippetSession {
    pub fn from_expanded(origin_byte_offset: usize, mut expanded: ExpandedSnippet) -> Option<Self> {
        if expanded.tabstops.is_empty() {
            return None;
        }
        for grp in &mut expanded.tabstops {
            for site in &mut grp.sites {
                site.start += origin_byte_offset;
                site.end += origin_byte_offset;
            }
        }
        Some(Self {
            tabstops: expanded.tabstops,
            active_group: 0,
            finished: false,
        })
    }

    pub fn active_group_ref(&self) -> Option<&TabstopGroup> {
        (!self.finished).then(|| self.tabstops.get(self.active_group)).flatten()
    }

    pub fn active_primary_span(&self) -> Option<(usize, usize)> {
        self.active_group_ref()?
            .sites
            .first()
            .map(|s| (s.start, s.end))
    }

    pub fn active_mirror_spans(&self) -> Vec<(usize, usize)> {
        let Some(grp) = self.active_group_ref() else {
            return Vec::new();
        };
        grp.sites.iter().skip(1).map(|s| (s.start, s.end)).collect()
    }

    /// Move to next tabstop (`Tab`). Returns `true` if another tabstop became active,
    /// or `false` once `$0` / end of snippet is reached (marking the session finished).
    pub fn advance_tabstop(&mut self) -> bool {
        if self.finished {
            return false;
        }
        if let Some(cur) = self.tabstops.get(self.active_group) {
            if cur.index == 0 {
                self.finished = true;
                return false;
            }
        }
        if self.active_group + 1 < self.tabstops.len() {
            self.active_group += 1;
            true
        } else {
            self.finished = true;
            false
        }
    }

    /// Move to previous tabstop (`Shift+Tab`).
    pub fn prev_tabstop(&mut self) -> bool {
        if self.finished || self.active_group == 0 {
            return false;
        }
        self.active_group -= 1;
        true
    }

    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Update the active tabstop primary site and synchronize all linked mirrors/transforms.
    /// Returns mirror/transform edits sorted in descending `(start, end, replacement)` order
    /// so applying them sequentially preserves earlier offsets.
    pub fn update_active_text(
        &mut self,
        new_primary_value: &str,
        primary_delta: isize,
    ) -> Vec<(usize, usize, String)> {
        if self.finished || self.active_group >= self.tabstops.len() {
            return Vec::new();
        }
        let primary_old_start = match self.tabstops[self.active_group].sites.first() {
            Some(s) => s.start,
            None => return Vec::new(),
        };
        let primary_new_end = (primary_old_start as isize
            + (self.tabstops[self.active_group].sites[0].end - primary_old_start) as isize
            + primary_delta)
            .max(primary_old_start as isize) as usize;

        // First collect mirror sites (referencing old offsets before shifting mirrors)
        let mut pending_mirror_edits: Vec<(usize, isize, usize, usize, String)> = Vec::new();
        for (site_idx, site) in self.tabstops[self.active_group]
            .sites
            .iter()
            .enumerate()
            .skip(1)
        {
            // Adjust offset if this mirror sits after the primary site
            let shift_from_primary = if site.start >= primary_old_start {
                primary_delta
            } else {
                0
            };
            let s_start = (site.start as isize + shift_from_primary).max(0) as usize;
            let s_end = (site.end as isize + shift_from_primary).max(s_start as isize) as usize;
            let replacement = if let Some(tr) = &site.transform {
                apply_snippet_transform(new_primary_value, tr)
            } else {
                new_primary_value.to_string()
            };
            let old_len = s_end.saturating_sub(s_start);
            let m_delta = replacement.len() as isize - old_len as isize;
            pending_mirror_edits.push((site_idx, m_delta, s_start, s_end, replacement));
        }

        // Update primary site's own end & shift every site after `primary_old_start`
        self.shift_sites_after(primary_old_start, primary_delta);
        self.tabstops[self.active_group].sites[0].end = primary_new_end;

        // Apply shifts produced by mirror edits sequentially in descending offset order
        // so returned buffer edits never invalidate each other.
        let mut out = Vec::with_capacity(pending_mirror_edits.len());
        pending_mirror_edits.sort_by_key(|a| std::cmp::Reverse(a.2));
        for (site_idx, m_delta, s_start, s_end, replacement) in pending_mirror_edits {
            self.shift_sites_after(s_start, m_delta);
            self.tabstops[self.active_group].sites[site_idx].start = s_start;
            self.tabstops[self.active_group].sites[site_idx].end = s_start + replacement.len();
            out.push((s_start, s_end, replacement));
        }
        out
    }

    fn shift_sites_after(&mut self, boundary_offset: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for grp in &mut self.tabstops {
            for site in &mut grp.sites {
                if site.start > boundary_offset {
                    site.start = (site.start as isize + delta).max(0) as usize;
                    site.end = (site.end as isize + delta).max(site.start as isize) as usize;
                } else if site.end > boundary_offset && site.start <= boundary_offset {
                    site.end = (site.end as isize + delta).max(site.start as isize) as usize;
                }
            }
        }
    }
}

type RawSnippetSiteEntry = (usize, usize, Option<SnippetTransform>, Vec<String>);
type RawSnippetSiteMap = BTreeMap<u32, Vec<RawSnippetSiteEntry>>;

pub fn expand_snippet_with_context(template: &str, ctx: &SnippetContext) -> ExpandedSnippet {
    let nodes = parse_snippet(template);
    let mut text = String::new();
    let mut defaults_by_index: HashMap<u32, String> = HashMap::new();
    let mut raw_sites: RawSnippetSiteMap = BTreeMap::new();

    #[allow(clippy::too_many_arguments)]
    fn render_nodes(
        nodes: &[SnippetNode],
        ctx: &SnippetContext,
        text: &mut String,
        defaults_by_index: &mut HashMap<u32, String>,
        raw_sites: &mut RawSnippetSiteMap,
    ) {
        for node in nodes {
            match node {
                SnippetNode::Text(s) => text.push_str(s),
                SnippetNode::Variable {
                    name,
                    fallback,
                    transform,
                } => {
                    let resolved = ctx.resolve_variable(name).filter(|v| !v.is_empty());
                    let base = if let Some(val) = resolved {
                        val
                    } else {
                        let mut fb = String::new();
                        render_nodes(fallback, ctx, &mut fb, defaults_by_index, raw_sites);
                        fb
                    };
                    let final_val = if let Some(tr) = transform {
                        apply_snippet_transform(&base, tr)
                    } else {
                        base
                    };
                    text.push_str(&final_val);
                }
                SnippetNode::Tabstop {
                    index,
                    placeholder,
                    transform,
                    choices,
                } => {
                    let start = text.len();
                    if !choices.is_empty() {
                        text.push_str(&choices[0]);
                        defaults_by_index
                            .entry(*index)
                            .or_insert_with(|| choices[0].clone());
                    } else if !placeholder.is_empty() {
                        let before = text.len();
                        render_nodes(placeholder, ctx, text, defaults_by_index, raw_sites);
                        let captured = text[before..].to_string();
                        defaults_by_index.entry(*index).or_insert(captured);
                    } else if let Some(tr) = transform {
                        let src = defaults_by_index.get(index).map_or("", String::as_str);
                        text.push_str(&apply_snippet_transform(src, tr));
                    } else if transform.is_none() {
                        if let Some(prev) = defaults_by_index.get(index) {
                            text.push_str(prev);
                        }
                    }
                    let end = text.len();
                    raw_sites.entry(*index).or_default().push((
                        start,
                        end,
                        transform.clone(),
                        choices.clone(),
                    ));
                }
            }
        }
    }

    render_nodes(&nodes, ctx, &mut text, &mut defaults_by_index, &mut raw_sites);

    let mut ordered_indices: Vec<u32> = raw_sites.keys().copied().filter(|&i| i > 0).collect();
    ordered_indices.sort_unstable();
    if raw_sites.contains_key(&0) {
        ordered_indices.push(0);
    }

    let tabstops = ordered_indices
        .into_iter()
        .filter_map(|idx| {
            let list = raw_sites.remove(&idx)?;
            let mut choices = Vec::new();
            let sites = list
                .into_iter()
                .enumerate()
                .map(|(pos, (start, end, transform, c))| {
                    if choices.is_empty() && !c.is_empty() {
                        choices = c;
                    }
                    TabstopSite {
                        start,
                        end,
                        is_mirror: pos > 0,
                        transform,
                    }
                })
                .collect();
            Some(TabstopGroup {
                index: idx,
                sites,
                choices,
                current_choice: 0,
            })
        })
        .collect();

    ExpandedSnippet { text, tabstops }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetDefinition {
    pub name: String,
    pub prefixes: Vec<String>,
    pub body: String,
    pub description: Option<String>,
    pub scope: Option<String>,
}

/// Registry holding snippets per language identifier loaded from extension `.json` snippet files.
#[derive(Debug, Clone, Default)]
pub struct SnippetRegistry {
    by_language: HashMap<String, Vec<SnippetDefinition>>,
}

pub fn strip_jsonc_comments_and_trailing_commas(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && matches!(chars[j], '}' | ']') {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

impl SnippetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_vscode_json(&mut self, language_id: &str, json_str: &str) -> Result<usize, String> {
        let cleaned = strip_jsonc_comments_and_trailing_commas(json_str);
        let root: Value = serde_json::from_str(&cleaned)
            .map_err(|e| format!("Failed parsing snippets JSON for '{}': {}", language_id, e))?;
        let Some(map) = root.as_object() else {
            return Err("Snippets file root must be a JSON object".to_string());
        };
        let list = self.by_language.entry(language_id.to_string()).or_default();
        let mut added = 0;
        for (name, val) in map {
            let Some(obj) = val.as_object() else {
                continue;
            };
            let prefixes = match obj.get("prefix") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect(),
                _ => continue,
            };
            let body = match obj.get("body") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => continue,
            };
            let description = obj
                .get("description")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let scope = obj
                .get("scope")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            list.push(SnippetDefinition {
                name: name.clone(),
                prefixes,
                body,
                description,
                scope,
            });
            added += 1;
        }
        Ok(added)
    }

    pub fn snippets_for_language(&self, language_id: &str) -> &[SnippetDefinition] {
        self.by_language
            .get(language_id)
            .map_or(&[], Vec::as_slice)
    }

    pub fn matching_prefix(&self, language_id: &str, prefix: &str) -> Vec<&SnippetDefinition> {
        self.snippets_for_language(language_id)
            .iter()
            .filter(|s| s.prefixes.iter().any(|p| p.starts_with(prefix)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snippet_expansion_mirrors_choices_variables_and_transforms() {
        let ctx = SnippetContext::from_editor(
            Some(Path::new("/src/user_service.rs")),
            9,
            "    ",
            "hello",
            "hello",
            "clip",
        );
        let tpl = "pub fn ${1:my_handler}(val: ${2|i32,u64,String|}) {\n    // file: ${TM_FILENAME_BASE/(_)/-/g} mirror: ${1/(.*)/${1:/upcase}/}\n    $0\n}";
        let exp = expand_snippet_with_context(tpl, &ctx);
        assert!(exp.text.contains("pub fn my_handler(val: i32)"));
        assert!(exp.text.contains("// file: user-service mirror: MY_HANDLER"));
        assert_eq!(exp.tabstops.len(), 3); // 1, 2, 0
        assert_eq!(exp.tabstops[0].index, 1);
        assert_eq!(exp.tabstops[0].sites.len(), 2); // primary + transformed mirror
        assert_eq!(exp.tabstops[1].index, 2);
        assert_eq!(exp.tabstops[1].choices, vec!["i32", "u64", "String"]);
        assert_eq!(exp.tabstops[2].index, 0);

        let mut session = SnippetSession::from_expanded(10, exp).unwrap();
        let (p_start, p_end) = session.active_primary_span().unwrap();
        assert_eq!(p_start, 10 + 7);
        assert_eq!(p_end, 10 + 7 + "my_handler".len());
        let edits = session.update_active_text("calc_sum", 8 - 10);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].2, "CALC_SUM");
        assert!(session.advance_tabstop());
        assert_eq!(session.active_group_ref().unwrap().index, 2);
        assert!(session.advance_tabstop()); // -> $0
        assert_eq!(session.active_group_ref().unwrap().index, 0);
        assert!(!session.advance_tabstop()); // finishes
        assert!(session.finished);
    }

    #[test]
    fn test_snippet_registry_vscode_jsonc() {
        let mut reg = SnippetRegistry::new();
        let count = reg
            .load_vscode_json(
                "rust",
                r##"{
                    // Line comment allowed
                    "Test Function": {
                        "prefix": ["tfn", "testfn"],
                        "body": [
                            "#[test]",
                            "fn ${1:test_name}() {",
                            "    $0",
                            "}"
                        ],
                        "description": "Create a unit test",
                    },
                }"##,
            )
            .unwrap();
        assert_eq!(count, 1);
        let hits = reg.matching_prefix("rust", "tf");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].body.contains("#[test]"));
    }
}
