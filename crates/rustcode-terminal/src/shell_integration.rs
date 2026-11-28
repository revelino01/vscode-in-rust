//! Terminal shell integration parser and state tracking for VS Code OSC 633 sequences.
//!
//! Recognizes:
//! - `OSC 633 ; A ST` -> Prompt start
//! - `OSC 633 ; B ST` -> Command input start
//! - `OSC 633 ; C ST` -> Command execution start
//! - `OSC 633 ; D [; exitCode] ST` -> Command finished
//! - `OSC 633 ; E ; commandLine [; nonce] ST` -> Explicit command line
//! - `OSC 633 ; F ST` / `G ST` -> Continuation prompt start/end
//! - `OSC 633 ; H ST` / `I ST` -> Right prompt start/end
//! - `OSC 633 ; P ; Key=Value ST` -> Property (e.g. `Cwd=...`)
//!
//! Both `BEL` (`\x07`) and `ST` (`ESC \`) string terminators are supported across arbitrary
//! byte chunk boundaries.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShellIntegrationEvent {
    PromptStart,
    CommandStart,
    CommandExecuted,
    CommandFinished { exit_code: Option<i32> },
    CommandLine { command_line: String },
    ContinuationStart,
    ContinuationEnd,
    RightPromptStart,
    RightPromptEnd,
    Property { name: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellExecutionRecord {
    pub command_line: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
    pub in_progress: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalShellIntegrationState {
    pub active: bool,
    pub cwd: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub current_command: Option<ShellExecutionRecord>,
    pub executions: Vec<ShellExecutionRecord>,
    #[serde(skip)]
    pending_command_line: Option<String>,
    #[serde(skip)]
    recent_events: VecDeque<ShellIntegrationEvent>,
}

impl TerminalShellIntegrationState {
    pub fn apply_event(&mut self, event: ShellIntegrationEvent) {
        self.active = true;
        match &event {
            ShellIntegrationEvent::PromptStart => {}
            ShellIntegrationEvent::CommandStart => {}
            ShellIntegrationEvent::CommandLine { command_line } => {
                self.pending_command_line = Some(command_line.clone());
                if let Some(cur) = self.current_command.as_mut() {
                    if cur.command_line.is_empty() {
                        cur.command_line = command_line.clone();
                    }
                }
            }
            ShellIntegrationEvent::CommandExecuted => {
                let command_line = self.pending_command_line.take().unwrap_or_default();
                if let Some(prev) = self.current_command.take() {
                    self.executions.push(prev);
                }
                self.current_command = Some(ShellExecutionRecord {
                    command_line,
                    cwd: self.cwd.clone(),
                    exit_code: None,
                    output: String::new(),
                    in_progress: true,
                });
            }
            ShellIntegrationEvent::CommandFinished { exit_code } => {
                if let Some(mut cur) = self.current_command.take() {
                    cur.exit_code = *exit_code;
                    cur.in_progress = false;
                    self.executions.push(cur);
                } else if let Some(cmd) = self.pending_command_line.take() {
                    self.executions.push(ShellExecutionRecord {
                        command_line: cmd,
                        cwd: self.cwd.clone(),
                        exit_code: *exit_code,
                        output: String::new(),
                        in_progress: false,
                    });
                }
            }
            ShellIntegrationEvent::Property { name, value } => {
                self.properties.insert(name.clone(), value.clone());
                if name.eq_ignore_ascii_case("cwd") {
                    self.cwd = Some(value.clone());
                    if let Some(cur) = self.current_command.as_mut() {
                        if cur.cwd.is_none() {
                            cur.cwd = Some(value.clone());
                        }
                    }
                }
            }
            ShellIntegrationEvent::ContinuationStart
            | ShellIntegrationEvent::ContinuationEnd
            | ShellIntegrationEvent::RightPromptStart
            | ShellIntegrationEvent::RightPromptEnd => {}
        }
        self.recent_events.push_back(event);
    }

    pub fn append_output_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(cur) = self.current_command.as_mut() {
            if cur.in_progress {
                cur.output.push_str(text);
            }
        }
    }

    pub fn drain_events(&mut self) -> Vec<ShellIntegrationEvent> {
        self.recent_events.drain(..).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Ground,
    Esc,
    OscPayload,
    OscEsc,
}

/// Incremental byte parser extracting `OSC 633` events while capturing plain text segments
/// between `CommandExecuted` and `CommandFinished`.
#[derive(Debug, Clone)]
pub struct Osc633Parser {
    state: ParserState,
    osc_buf: Vec<u8>,
}

impl Default for Osc633Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Osc633Parser {
    const MAX_OSC_LEN: usize = 16 * 1024;

    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            osc_buf: Vec::new(),
        }
    }

    pub fn feed_bytes(
        &mut self,
        bytes: &[u8],
        state: &mut TerminalShellIntegrationState,
    ) {
        let mut plain_buf: Vec<u8> = Vec::new();
        for &b in bytes {
            match self.state {
                ParserState::Ground => {
                    if b == 0x1B {
                        self.state = ParserState::Esc;
                    } else {
                        if state.current_command.is_some() {
                            plain_buf.push(b);
                        }
                    }
                }
                ParserState::Esc => {
                    if b == b']' {
                        if !plain_buf.is_empty() {
                            state.append_output_text(&String::from_utf8_lossy(&plain_buf));
                            plain_buf.clear();
                        }
                        self.osc_buf.clear();
                        self.state = ParserState::OscPayload;
                    } else {
                        if state.current_command.is_some() {
                            plain_buf.push(0x1B);
                            plain_buf.push(b);
                        }
                        self.state = ParserState::Ground;
                    }
                }
                ParserState::OscPayload => {
                    if b == 0x07 {
                        let raw = std::mem::take(&mut self.osc_buf);
                        self.state = ParserState::Ground;
                        if let Some(ev) = parse_osc633_payload(&raw) {
                            state.apply_event(ev);
                        }
                    } else if b == 0x1B {
                        self.state = ParserState::OscEsc;
                    } else if self.osc_buf.len() < Self::MAX_OSC_LEN {
                        self.osc_buf.push(b);
                    }
                }
                ParserState::OscEsc => {
                    if b == b'\\' {
                        let raw = std::mem::take(&mut self.osc_buf);
                        self.state = ParserState::Ground;
                        if let Some(ev) = parse_osc633_payload(&raw) {
                            state.apply_event(ev);
                        }
                    } else if b == b']' {
                        // Interrupted by a new OSC sequence.
                        self.osc_buf.clear();
                        self.state = ParserState::OscPayload;
                    } else {
                        self.state = ParserState::OscPayload;
                    }
                }
            }
        }
        if !plain_buf.is_empty() {
            state.append_output_text(&String::from_utf8_lossy(&plain_buf));
        }
    }
}

fn parse_osc633_payload(raw: &[u8]) -> Option<ShellIntegrationEvent> {
    let s = std::str::from_utf8(raw).ok()?;
    let rest = s.strip_prefix("633;")?;
    let mut parts = rest.splitn(3, ';');
    let code = parts.next()?;
    let arg1 = parts.next();
    let _arg2 = parts.next();

    match code {
        "A" => Some(ShellIntegrationEvent::PromptStart),
        "B" => Some(ShellIntegrationEvent::CommandStart),
        "C" => Some(ShellIntegrationEvent::CommandExecuted),
        "D" => Some(ShellIntegrationEvent::CommandFinished {
            exit_code: arg1.and_then(|v| v.trim().parse::<i32>().ok()),
        }),
        "E" => {
            let cmd = unescape_osc633_value(arg1.unwrap_or_default());
            Some(ShellIntegrationEvent::CommandLine { command_line: cmd })
        }
        "F" => Some(ShellIntegrationEvent::ContinuationStart),
        "G" => Some(ShellIntegrationEvent::ContinuationEnd),
        "H" => Some(ShellIntegrationEvent::RightPromptStart),
        "I" => Some(ShellIntegrationEvent::RightPromptEnd),
        "P" => {
            let kv = arg1?;
            let (name, value) = kv.split_once('=')?;
            Some(ShellIntegrationEvent::Property {
                name: name.to_string(),
                value: unescape_osc633_value(value),
            })
        }
        _ => None,
    }
}

fn unescape_osc633_value(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'x' if i + 3 < bytes.len() => {
                    if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 2..i + 4]) {
                        if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                            out.push(val as char);
                            i += 4;
                            continue;
                        }
                    }
                    out.push('\\');
                    i += 1;
                }
                b'n' => {
                    out.push('\n');
                    i += 2;
                }
                b'r' => {
                    out.push('\r');
                    i += 2;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                }
                _ => {
                    out.push('\\');
                    i += 1;
                }
            }
        } else {
            let ch = input[i..].chars().next().unwrap_or('\0');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc633_chunked_sequences_and_execution_history() {
        let mut parser = Osc633Parser::new();
        let mut state = TerminalShellIntegrationState::default();

        // Feed across arbitrary chunk boundaries using both BEL and ST terminators
        let stream = concat!(
            "\x1b]633;A\x07",
            "user@host$ ",
            "\x1b]633;P;Cwd=/workspace/app\x1b\\",
            "\x1b]633;B\x07",
            "\x1b]633;E;cargo test\\x3b echo ok\x07",
            "\x1b]633;C\x07",
            "running 12 tests\n",
            "\x1b]633;D;0\x1b\\",
        );

        for chunk in stream.as_bytes().chunks(5) {
            parser.feed_bytes(chunk, &mut state);
        }

        assert!(state.active);
        assert_eq!(state.cwd.as_deref(), Some("/workspace/app"));
        assert!(state.current_command.is_none());
        assert_eq!(state.executions.len(), 1);
        let rec = &state.executions[0];
        assert_eq!(rec.command_line, "cargo test; echo ok");
        assert_eq!(rec.cwd.as_deref(), Some("/workspace/app"));
        assert_eq!(rec.exit_code, Some(0));
        assert!(!rec.in_progress);
        assert!(rec.output.contains("running 12 tests"));

        let events = state.drain_events();
        assert_eq!(events.len(), 6);
        assert!(matches!(
            events.last(),
            Some(ShellIntegrationEvent::CommandFinished { exit_code: Some(0) })
        ));
    }
}
