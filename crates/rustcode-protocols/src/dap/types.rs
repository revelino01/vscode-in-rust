//! Debug Adapter Protocol (DAP) types and JSON structures.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level DAP message conforming to DAP specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum DapMessage {
    #[serde(rename = "request")]
    Request {
        seq: i64,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<Value>,
    },
    #[serde(rename = "response")]
    Response {
        seq: i64,
        request_seq: i64,
        success: bool,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Value>,
    },
    #[serde(rename = "event")]
    Event {
        seq: i64,
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequestArguments {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    pub adapter_id: String,
    #[serde(default)]
    pub lines_start_at1: Option<bool>,
    #[serde(default)]
    pub columns_start_at1: Option<bool>,
    #[serde(default)]
    pub path_format: Option<String>,
    #[serde(default)]
    pub supports_variable_type: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub supports_configuration_done_request: Option<bool>,
    #[serde(default)]
    pub supports_function_breakpoints: Option<bool>,
    #[serde(default)]
    pub supports_conditional_breakpoints: Option<bool>,
    #[serde(default)]
    pub supports_hit_conditional_breakpoints: Option<bool>,
    #[serde(default)]
    pub supports_evaluate_for_hovers: Option<bool>,
    #[serde(default)]
    pub supports_step_back: Option<bool>,
    #[serde(default)]
    pub supports_restart_frame: Option<bool>,
    #[serde(default)]
    pub supports_restart_request: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub source_reference: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

impl SourceBreakpoint {
    pub fn new(line: u32) -> Self {
        Self {
            line,
            column: None,
            condition: None,
            hit_condition: None,
            log_message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Breakpoint {
    #[serde(default)]
    pub id: Option<i64>,
    pub verified: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub source: Option<Source>,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub variables_reference: u32,
    #[serde(default)]
    pub expensive: bool,
    #[serde(default)]
    pub named_variables: Option<u32>,
    #[serde(default)]
    pub indexed_variables: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    pub variables_reference: u32,
    #[serde(default)]
    pub evaluate_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoppedEventBody {
    pub reason: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub all_threads_stopped: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutputEventBody {
    #[serde(default)]
    pub category: Option<String>,
    pub output: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
    #[serde(default)]
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContinuedEventBody {
    pub thread_id: i64,
    #[serde(default)]
    pub all_threads_continued: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsArguments {
    pub source: Source,
    pub breakpoints: Vec<SourceBreakpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsResponseBody {
    pub breakpoints: Vec<Breakpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsResponseBody {
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceResponseBody {
    pub stack_frames: Vec<StackFrame>,
    #[serde(default)]
    pub total_frames: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopesResponseBody {
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesResponseBody {
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResponseBody {
    pub result: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    pub variables_reference: u32,
}
