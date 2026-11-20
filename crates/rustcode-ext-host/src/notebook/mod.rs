//! Extension Host Notebook Manager & RPC bridge DTOs for `vscode.notebooks.*`.

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ExtNotebookCellKind;

impl ExtNotebookCellKind {
    pub const MARKUP: u8 = 1;
    pub const CODE: u8 = 2;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtNotebookCellOutputItem {
    pub mime: String,
    pub data: Vec<u8>,
}

impl ExtNotebookCellOutputItem {
    pub fn text(value: impl Into<String>, mime: impl Into<String>) -> Self {
        Self {
            mime: mime.into(),
            data: value.into().into_bytes(),
        }
    }

    pub fn stdout(value: impl Into<String>) -> Self {
        Self::text(value, "application/vnd.code.notebook.stdout")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtNotebookCellOutput {
    #[serde(default)]
    pub items: Vec<ExtNotebookCellOutputItem>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtNotebookCellData {
    pub kind: u8,
    pub value: String,
    #[serde(default)]
    pub language_id: String,
    #[serde(default)]
    pub outputs: Vec<ExtNotebookCellOutput>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub execution_summary: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExtNotebookData {
    #[serde(default)]
    pub cells: Vec<ExtNotebookCellData>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookSerializerInfo {
    pub id: String,
    pub notebook_type: String,
    pub options: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookControllerInfo {
    pub id: String,
    pub notebook_type: String,
    pub label: String,
    pub supported_languages: Vec<String>,
    pub supports_execution_order: bool,
    pub is_disposed: bool,
    pub affinities: HashMap<String, i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookCellExecutionState {
    pub execution_id: String,
    pub controller_id: String,
    pub notebook_uri: String,
    pub cell_index: usize,
    pub state: String,
    pub execution_order: Option<u32>,
    pub start_time: Option<u64>,
    pub end_time: Option<u64>,
    pub outputs: Vec<ExtNotebookCellOutput>,
}

pub type NotebookEventListener = Arc<dyn Fn(String, Value) + Send + Sync>;
type CellOutputMap = HashMap<(String, usize), Vec<ExtNotebookCellOutput>>;

#[derive(Clone, Default)]
pub struct ExtHostNotebookManager {
    serializers: Arc<RwLock<HashMap<String, NotebookSerializerInfo>>>,
    controllers: Arc<RwLock<HashMap<String, NotebookControllerInfo>>>,
    executions: Arc<RwLock<HashMap<(String, usize), NotebookCellExecutionState>>>,
    cell_outputs: Arc<RwLock<CellOutputMap>>,
    listeners: Arc<RwLock<Vec<NotebookEventListener>>>,
}

impl ExtHostNotebookManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_notebook_event<F>(&self, listener: F)
    where
        F: Fn(String, Value) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit_event(&self, event_name: &str, payload: Value) {
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(event_name.to_string(), payload.clone());
        }
    }

    pub fn register_serializer(
        &self,
        id: impl Into<String>,
        notebook_type: impl Into<String>,
        options: Value,
    ) {
        let id_str = id.into();
        let info = NotebookSerializerInfo {
            id: id_str.clone(),
            notebook_type: notebook_type.into(),
            options,
        };
        self.serializers.write().insert(id_str, info.clone());
        self.emit_event(
            "notebooks.registerSerializer",
            serde_json::to_value(info).unwrap_or(Value::Null),
        );
    }

    pub fn unregister_serializer(&self, id: &str) -> Option<NotebookSerializerInfo> {
        let removed = self.serializers.write().remove(id);
        if removed.is_some() {
            self.emit_event("notebooks.unregisterSerializer", json!({ "id": id }));
        }
        removed
    }

    pub fn get_serializer(&self, id: &str) -> Option<NotebookSerializerInfo> {
        self.serializers.read().get(id).cloned()
    }

    pub fn list_serializers(&self) -> Vec<NotebookSerializerInfo> {
        self.serializers.read().values().cloned().collect()
    }

    pub fn create_controller(
        &self,
        id: impl Into<String>,
        notebook_type: impl Into<String>,
        label: impl Into<String>,
        supported_languages: Vec<String>,
        supports_execution_order: bool,
    ) {
        let id_str = id.into();
        let info = NotebookControllerInfo {
            id: id_str.clone(),
            notebook_type: notebook_type.into(),
            label: label.into(),
            supported_languages,
            supports_execution_order,
            is_disposed: false,
            affinities: HashMap::new(),
        };
        self.controllers.write().insert(id_str, info.clone());
        self.emit_event(
            "notebooks.createController",
            serde_json::to_value(info).unwrap_or(Value::Null),
        );
    }

    pub fn dispose_controller(&self, id: &str) -> bool {
        let mut guard = self.controllers.write();
        if let Some(ctrl) = guard.get_mut(id) {
            ctrl.is_disposed = true;
            drop(guard);
            self.emit_event("notebooks.disposeController", json!({ "id": id }));
            true
        } else {
            false
        }
    }

    pub fn get_controller(&self, id: &str) -> Option<NotebookControllerInfo> {
        self.controllers.read().get(id).cloned()
    }

    pub fn list_controllers(&self) -> Vec<NotebookControllerInfo> {
        self.controllers
            .read()
            .values()
            .filter(|c| !c.is_disposed)
            .cloned()
            .collect()
    }

    pub fn get_execution(
        &self,
        notebook_uri: &str,
        cell_index: usize,
    ) -> Option<NotebookCellExecutionState> {
        self.executions
            .read()
            .get(&(notebook_uri.to_string(), cell_index))
            .cloned()
    }

    pub fn get_cell_outputs(
        &self,
        notebook_uri: &str,
        cell_index: usize,
    ) -> Vec<ExtNotebookCellOutput> {
        self.cell_outputs
            .read()
            .get(&(notebook_uri.to_string(), cell_index))
            .cloned()
            .unwrap_or_default()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "notebooks.registerSerializer" => {
                let id = params
                    .get("id")
                    .or_else(|| params.get("notebookType"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing serializer id".to_string())?;
                let notebook_type = params
                    .get("notebookType")
                    .and_then(|v| v.as_str())
                    .unwrap_or(id);
                let options = params.get("options").cloned().unwrap_or(json!({}));
                self.register_serializer(id, notebook_type, options);
                Ok(json!({ "registered": true, "id": id }))
            }
            "notebooks.unregisterSerializer" => {
                let id = params
                    .get("id")
                    .or_else(|| params.get("notebookType"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing serializer id".to_string())?;
                let removed = self.unregister_serializer(id).is_some();
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "notebooks.createController" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controller id".to_string())?;
                let notebook_type = params
                    .get("notebookType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("jupyter-notebook");
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(id);
                let supported_languages: Vec<String> = match params.get("supportedLanguages") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid supportedLanguages: {}", e))?,
                    _ => Vec::new(),
                };
                let supports_execution_order = params
                    .get("supportsExecutionOrder")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.create_controller(
                    id,
                    notebook_type,
                    label,
                    supported_languages,
                    supports_execution_order,
                );
                Ok(json!({ "created": true, "id": id }))
            }
            "notebooks.disposeController" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controller id".to_string())?;
                let disposed = self.dispose_controller(id);
                Ok(json!({ "disposed": disposed, "id": id }))
            }
            "notebooks.updateExecution" => {
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                let notebook_uri = params
                    .get("notebookUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cell_index = params
                    .get("cellIndex")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let raw_state = params
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("start");
                let mapped_state = match raw_state {
                    "start" => "running",
                    "end" => {
                        if params.get("success").and_then(|v| v.as_bool()).unwrap_or(true) {
                            "succeeded"
                        } else {
                            "failed"
                        }
                    }
                    other => other,
                }
                .to_string();
                let execution_order = params
                    .get("executionOrder")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32);
                let start_time = params.get("startTime").and_then(|v| v.as_u64());
                let end_time = params.get("endTime").and_then(|v| v.as_u64());

                let key = (notebook_uri.clone(), cell_index);
                let outputs = self
                    .cell_outputs
                    .read()
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();

                let mut guard = self.executions.write();
                let entry = guard.entry(key).or_insert_with(|| NotebookCellExecutionState {
                    execution_id: format!("{}:{}:{}", controller_id, notebook_uri, cell_index),
                    controller_id: controller_id.clone(),
                    notebook_uri: notebook_uri.clone(),
                    cell_index,
                    state: mapped_state.clone(),
                    execution_order,
                    start_time,
                    end_time,
                    outputs: outputs.clone(),
                });
                entry.controller_id = controller_id;
                entry.state = mapped_state;
                if execution_order.is_some() {
                    entry.execution_order = execution_order;
                }
                if start_time.is_some() {
                    entry.start_time = start_time;
                }
                if raw_state == "start" {
                    entry.end_time = None;
                }
                if end_time.is_some() {
                    entry.end_time = end_time;
                }
                entry.outputs = outputs;
                let snapshot = entry.clone();
                drop(guard);

                self.emit_event(
                    "notebooks.updateExecution",
                    serde_json::to_value(&snapshot).unwrap_or(Value::Null),
                );
                Ok(json!({ "updated": true, "execution": snapshot }))
            }
            "notebooks.updateCellOutputs" => {
                let notebook_uri = params
                    .get("notebookUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cell_index = params
                    .get("cellIndex")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let outputs: Vec<ExtNotebookCellOutput> = match params.get("outputs") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid outputs: {}", e))?,
                    _ => Vec::new(),
                };
                let key = (notebook_uri.clone(), cell_index);
                self.cell_outputs.write().insert(key.clone(), outputs.clone());
                if let Some(exec) = self.executions.write().get_mut(&key) {
                    exec.outputs = outputs.clone();
                }
                self.emit_event(
                    "notebooks.updateCellOutputs",
                    json!({
                        "notebookUri": notebook_uri,
                        "cellIndex": cell_index,
                        "outputs": outputs,
                    }),
                );
                Ok(json!({ "updated": true, "count": outputs.len() }))
            }
            "notebooks.appendCellOutputs" => {
                let notebook_uri = params
                    .get("notebookUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cell_index = params
                    .get("cellIndex")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let extra: Vec<ExtNotebookCellOutput> = match params.get("outputs") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid outputs: {}", e))?,
                    _ => Vec::new(),
                };
                let key = (notebook_uri.clone(), cell_index);
                let total_len = {
                    let mut guard = self.cell_outputs.write();
                    let list = guard.entry(key.clone()).or_default();
                    list.extend(extra.clone());
                    let len = list.len();
                    if let Some(exec) = self.executions.write().get_mut(&key) {
                        exec.outputs = list.clone();
                    }
                    len
                };
                self.emit_event(
                    "notebooks.appendCellOutputs",
                    json!({
                        "notebookUri": notebook_uri,
                        "cellIndex": cell_index,
                        "appended": extra,
                    }),
                );
                Ok(json!({ "appended": true, "count": total_len }))
            }
            "notebooks.updateControllerAffinity" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controller id".to_string())?;
                let notebook_uri = params
                    .get("notebookUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let affinity = params
                    .get("affinity")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1) as i32;
                {
                    let mut guard = self.controllers.write();
                    let ctrl = guard
                        .get_mut(id)
                        .ok_or_else(|| format!("Notebook controller '{}' not found", id))?;
                    if ctrl.is_disposed {
                        return Err(format!("Notebook controller '{}' is disposed", id));
                    }
                    ctrl.affinities.insert(notebook_uri.clone(), affinity);
                }
                self.emit_event(
                    "notebooks.updateControllerAffinity",
                    json!({ "id": id, "notebookUri": notebook_uri, "affinity": affinity }),
                );
                Ok(json!({ "updated": true }))
            }
            _ => Err(format!("Unknown notebook RPC method: {}", method)),
        }
    }

    pub async fn request_deserialize(
        &self,
        rpc_bridge: &RpcBridge,
        notebook_type: &str,
        content: &[u8],
    ) -> Result<ExtNotebookData, String> {
        let val = rpc_bridge
            .request(
                "notebooks.deserializeNotebook",
                json!({
                    "providerId": notebook_type,
                    "notebookType": notebook_type,
                    "content": content,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'notebooks.deserializeNotebook' timed out") {
                    "Timed out waiting for deserializeNotebook".to_string()
                } else {
                    e
                }
            })?;

        serde_json::from_value::<ExtNotebookData>(val)
            .map_err(|e| format!("Invalid ExtNotebookData response: {}", e))
    }

    pub async fn request_serialize(
        &self,
        rpc_bridge: &RpcBridge,
        notebook_type: &str,
        data: &ExtNotebookData,
    ) -> Result<Vec<u8>, String> {
        let val = rpc_bridge
            .request(
                "notebooks.serializeNotebook",
                json!({
                    "providerId": notebook_type,
                    "notebookType": notebook_type,
                    "data": data,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'notebooks.serializeNotebook' timed out") {
                    "Timed out waiting for serializeNotebook".to_string()
                } else {
                    e
                }
            })?;

        if let Ok(bytes) = serde_json::from_value::<Vec<u8>>(val.clone()) {
            Ok(bytes)
        } else if let Some(s) = val.as_str() {
            Ok(s.as_bytes().to_vec())
        } else {
            Err("Invalid serialized notebook payload".to_string())
        }
    }

    pub async fn request_execute_cells(
        &self,
        rpc_bridge: &RpcBridge,
        controller_id: &str,
        notebook_uri: &str,
        cell_indices: &[usize],
        cells: &[ExtNotebookCellData],
    ) -> Result<Value, String> {
        rpc_bridge
            .request(
                "notebooks.executeCells",
                json!({
                    "controllerId": controller_id,
                    "providerId": controller_id,
                    "notebook": { "uri": notebook_uri },
                    "cellIndices": cell_indices,
                    "cells": cells,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'notebooks.executeCells' timed out") {
                    "Timed out waiting for executeCells".to_string()
                } else {
                    e
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_host_notebook_manager_rpc_lifecycle() {
        let mgr = ExtHostNotebookManager::new();
        let events = Arc::new(RwLock::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_notebook_event(move |name, _| {
            events_clone.write().push(name);
        });

        let reg_res = mgr
            .handle_rpc_call(
                "notebooks.registerSerializer",
                &json!({ "notebookType": "jupyter-notebook", "options": { "transientOutputs": false } }),
            )
            .unwrap();
        assert_eq!(reg_res["registered"], true);
        assert_eq!(mgr.list_serializers().len(), 1);

        let ctrl_res = mgr
            .handle_rpc_call(
                "notebooks.createController",
                &json!({
                    "id": "rust-nb-ctrl",
                    "notebookType": "jupyter-notebook",
                    "label": "Pure-Rust Controller",
                    "supportedLanguages": ["python", "rust"],
                    "supportsExecutionOrder": true
                }),
            )
            .unwrap();
        assert_eq!(ctrl_res["created"], true);
        assert_eq!(mgr.list_controllers().len(), 1);

        mgr.handle_rpc_call(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-nb-ctrl",
                "notebookUri": "file:///test.ipynb",
                "cellIndex": 0,
                "state": "start",
                "executionOrder": 1
            }),
        )
        .unwrap();
        assert_eq!(
            mgr.get_execution("file:///test.ipynb", 0).unwrap().state,
            "running"
        );

        mgr.handle_rpc_call(
            "notebooks.updateCellOutputs",
            &json!({
                "notebookUri": "file:///test.ipynb",
                "cellIndex": 0,
                "outputs": [{
                    "items": [{ "mime": "text/plain", "data": [52, 50] }]
                }]
            }),
        )
        .unwrap();
        assert_eq!(mgr.get_cell_outputs("file:///test.ipynb", 0).len(), 1);

        mgr.handle_rpc_call(
            "notebooks.appendCellOutputs",
            &json!({
                "notebookUri": "file:///test.ipynb",
                "cellIndex": 0,
                "outputs": [{
                    "items": [{ "mime": "application/vnd.code.notebook.stdout", "data": [111, 107] }]
                }]
            }),
        )
        .unwrap();
        assert_eq!(mgr.get_cell_outputs("file:///test.ipynb", 0).len(), 2);

        mgr.handle_rpc_call(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-nb-ctrl",
                "notebookUri": "file:///test.ipynb",
                "cellIndex": 0,
                "state": "end",
                "success": true
            }),
        )
        .unwrap();
        assert_eq!(
            mgr.get_execution("file:///test.ipynb", 0).unwrap().state,
            "succeeded"
        );

        mgr.handle_rpc_call("notebooks.disposeController", &json!({ "id": "rust-nb-ctrl" }))
            .unwrap();
        assert!(mgr.list_controllers().is_empty());

        mgr.handle_rpc_call(
            "notebooks.unregisterSerializer",
            &json!({ "id": "jupyter-notebook" }),
        )
        .unwrap();
        assert!(mgr.list_serializers().is_empty());
        assert_eq!(events.read().len(), 8);
    }

    #[test]
    fn test_ext_host_notebook_reexecution_clears_prior_end_time() {
        let mgr = ExtHostNotebookManager::new();
        mgr.handle_rpc_call(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-nb-ctrl",
                "notebookUri": "file:///reexec.ipynb",
                "cellIndex": 0,
                "state": "start",
                "startTime": 100
            }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-nb-ctrl",
                "notebookUri": "file:///reexec.ipynb",
                "cellIndex": 0,
                "state": "end",
                "success": true,
                "endTime": 250
            }),
        )
        .unwrap();
        let finished = mgr.get_execution("file:///reexec.ipynb", 0).unwrap();
        assert_eq!(finished.state, "succeeded");
        assert_eq!(finished.end_time, Some(250));

        mgr.handle_rpc_call(
            "notebooks.updateExecution",
            &json!({
                "controllerId": "rust-nb-ctrl",
                "notebookUri": "file:///reexec.ipynb",
                "cellIndex": 0,
                "state": "start",
                "startTime": 300
            }),
        )
        .unwrap();
        let restarted = mgr.get_execution("file:///reexec.ipynb", 0).unwrap();
        assert_eq!(restarted.state, "running");
        assert_eq!(restarted.start_time, Some(300));
        assert_eq!(restarted.end_time, None);
    }
}
