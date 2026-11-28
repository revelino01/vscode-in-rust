//! Extension Host Task Manager (`vscode.tasks.*` and Problem Matchers).

pub mod problem_matcher;
pub use problem_matcher::{
    ProblemMatcherContribution, ProblemMatcherRegistry, ProblemPattern, ProblemPatternSpec,
};

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTaskProviderInfo {
    pub id: String,
    pub task_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTaskExecutionState {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub task_type: String,
    pub status: String,
    #[serde(default)]
    pub process_id: Option<u32>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub definition: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtTaskEvent {
    ProviderRegistered {
        id: String,
        task_type: String,
    },
    ProviderUnregistered {
        id: String,
    },
    TaskStarted {
        id: String,
        name: String,
        source: String,
    },
    TaskTerminated {
        id: String,
    },
    TaskProcessStarted {
        id: String,
        process_id: u32,
    },
    TaskProcessEnded {
        id: String,
        exit_code: i32,
    },
    CustomExecutionWrite {
        id: String,
        data: String,
    },
    CustomExecutionClose {
        id: String,
        exit_code: Option<i32>,
    },
}

pub type TaskEventListener = Arc<dyn Fn(&ExtTaskEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostTaskManager {
    providers: Arc<RwLock<HashMap<String, ExtTaskProviderInfo>>>,
    executions: Arc<RwLock<HashMap<String, ExtTaskExecutionState>>>,
    execution_outputs: Arc<RwLock<HashMap<String, String>>>,
    execution_order: Arc<RwLock<Vec<String>>>,
    listeners: Arc<RwLock<Vec<TaskEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtTaskEvent>>>,
    problem_matchers: Arc<ProblemMatcherRegistry>,
    revision: Arc<AtomicU64>,
    next_exec_seq: Arc<AtomicU64>,
}

impl ExtHostTaskManager {
    pub fn new() -> Self {
        Self {
            problem_matchers: Arc::new(ProblemMatcherRegistry::new()),
            ..Self::default()
        }
    }

    pub fn problem_matchers(&self) -> &Arc<ProblemMatcherRegistry> {
        &self.problem_matchers
    }

    pub fn execution_output(&self, id: &str) -> Option<String> {
        self.execution_outputs.read().get(id).cloned()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtTaskEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_task_event<F>(&self, listener: F)
    where
        F: Fn(&ExtTaskEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtTaskEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&event);
        }
    }

    pub fn list_providers(&self) -> Vec<ExtTaskProviderInfo> {
        self.providers.read().values().cloned().collect()
    }

    pub fn get_execution(&self, id: &str) -> Option<ExtTaskExecutionState> {
        self.executions.read().get(id).cloned()
    }

    pub fn list_active_executions(&self) -> Vec<ExtTaskExecutionState> {
        let map = self.executions.read();
        let order = self.execution_order.read();
        order
            .iter()
            .filter_map(|id| map.get(id))
            .filter(|exec| exec.status == "running")
            .cloned()
            .collect()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "tasks.registerProvider" => {
                let task_type = params
                    .get("type")
                    .or_else(|| params.get("taskType"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing task type".to_string())?
                    .to_string();
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("task_provider_{}", task_type));
                let info = ExtTaskProviderInfo {
                    id: id.clone(),
                    task_type: task_type.clone(),
                };
                self.providers.write().insert(id.clone(), info);
                self.emit(ExtTaskEvent::ProviderRegistered {
                    id: id.clone(),
                    task_type,
                });
                Ok(json!({ "registered": true, "id": id }))
            }
            "tasks.unregisterProvider" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing provider id".to_string())?;
                let removed = self.providers.write().remove(id).is_some();
                if removed {
                    self.emit(ExtTaskEvent::ProviderUnregistered { id: id.to_string() });
                }
                Ok(json!({ "unregistered": removed, "id": id }))
            }
            "tasks.fetchTasks" => {
                let type_filter = params
                    .get("filter")
                    .and_then(|f| f.get("type"))
                    .and_then(|v| v.as_str());
                let list: Vec<Value> = self
                    .providers
                    .read()
                    .values()
                    .filter(|p| type_filter.is_none_or(|tf| tf == p.task_type))
                    .map(|p| json!({ "providerId": p.id, "type": p.task_type }))
                    .collect();
                Ok(json!(list))
            }
            "tasks.executeTask" => {
                let task = params.get("task").cloned().unwrap_or(Value::Null);
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        let seq = self.next_exec_seq.fetch_add(1, Ordering::SeqCst) + 1;
                        format!("task_exec_{}", seq)
                    });
                let name = task
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Task")
                    .to_string();
                let source = task
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Workspace")
                    .to_string();
                let definition = task.get("definition").cloned().unwrap_or(Value::Null);
                let task_type = definition
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("shell")
                    .to_string();

                let state = ExtTaskExecutionState {
                    id: id.clone(),
                    name: name.clone(),
                    source: source.clone(),
                    task_type,
                    status: "running".to_string(),
                    process_id: None,
                    exit_code: None,
                    definition,
                };

                {
                    let mut map = self.executions.write();
                    let mut order = self.execution_order.write();
                    if !map.contains_key(&id) {
                        order.push(id.clone());
                    }
                    map.insert(id.clone(), state);
                }

                self.emit(ExtTaskEvent::TaskStarted {
                    id: id.clone(),
                    name,
                    source,
                });
                Ok(json!({ "executed": true, "id": id }))
            }
            "tasks.terminateTask" => {
                let id = params.get("id").and_then(|v| v.as_str());
                let name = params.get("name").and_then(|v| v.as_str());

                let resolved_id = {
                    let mut map = self.executions.write();
                    let target_key = if let Some(id_str) = id {
                        map.contains_key(id_str).then(|| id_str.to_string())
                    } else if let Some(n) = name {
                        map.values()
                            .find(|e| e.name == n && e.status == "running")
                            .map(|e| e.id.clone())
                    } else {
                        None
                    };
                    if let Some(ref key) = target_key {
                        if let Some(exec) = map.get_mut(key) {
                            if exec.status == "exited" || exec.status == "terminated" {
                                return Err(format!(
                                    "Task execution '{}' has already {}",
                                    key, exec.status
                                ));
                            }
                            exec.status = "terminated".to_string();
                        }
                    }
                    target_key
                };

                if let Some(terminated_id) = resolved_id {
                    self.emit(ExtTaskEvent::TaskTerminated {
                        id: terminated_id.clone(),
                    });
                    Ok(json!({ "terminated": true, "id": terminated_id }))
                } else {
                    Err("Task execution not found".to_string())
                }
            }
            "tasks.reportTaskProcessStart" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing task execution id".to_string())?;
                let process_id = params
                    .get("processId")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| "Missing processId".to_string())?
                    as u32;
                {
                    let mut map = self.executions.write();
                    let exec = map
                        .get_mut(id)
                        .ok_or_else(|| format!("Task execution '{}' not found", id))?;
                    exec.process_id = Some(process_id);
                }
                self.emit(ExtTaskEvent::TaskProcessStarted {
                    id: id.to_string(),
                    process_id,
                });
                Ok(json!({ "recorded": true, "id": id, "processId": process_id }))
            }
            "tasks.reportTaskProcessEnd" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing task execution id".to_string())?;
                let exit_code = params
                    .get("exitCode")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                {
                    let mut map = self.executions.write();
                    let exec = map
                        .get_mut(id)
                        .ok_or_else(|| format!("Task execution '{}' not found", id))?;
                    exec.exit_code = Some(exit_code);
                    exec.status = "exited".to_string();
                }
                self.emit(ExtTaskEvent::TaskProcessEnded {
                    id: id.to_string(),
                    exit_code,
                });
                Ok(json!({ "recorded": true, "id": id, "exitCode": exit_code }))
            }
            "tasks.writeCustomExecution" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing task execution id".to_string())?;
                let data = params
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                self.execution_outputs
                    .write()
                    .entry(id.to_string())
                    .or_default()
                    .push_str(data);
                self.emit(ExtTaskEvent::CustomExecutionWrite {
                    id: id.to_string(),
                    data: data.to_string(),
                });
                Ok(json!({ "written": true, "id": id }))
            }
            "tasks.closeCustomExecution" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing task execution id".to_string())?;
                let exit_code = params
                    .get("exitCode")
                    .and_then(|v| v.as_i64())
                    .map(|c| c as i32);
                {
                    let mut map = self.executions.write();
                    if let Some(exec) = map.get_mut(id) {
                        exec.exit_code = exit_code;
                        exec.status = "exited".to_string();
                    }
                }
                self.emit(ExtTaskEvent::CustomExecutionClose {
                    id: id.to_string(),
                    exit_code,
                });
                Ok(json!({ "closed": true, "id": id, "exitCode": exit_code }))
            }
            _ => Err(format!("Unsupported task RPC method: {}", method)),
        }
    }

    pub async fn request_provide_tasks(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
    ) -> Result<Vec<Value>, String> {
        let val = rpc_bridge
            .request(
                "tasks.provideTasks",
                json!({
                    "providerId": provider_id,
                    "taskType": provider_id,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'tasks.provideTasks' timed out") {
                    "Timed out waiting for provideTasks".to_string()
                } else {
                    e
                }
            })?;

        Ok(val.as_array().cloned().unwrap_or_default())
    }

    pub async fn request_resolve_task(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        task: Value,
    ) -> Result<Value, String> {
        rpc_bridge
            .request(
                "tasks.resolveTask",
                json!({
                    "providerId": provider_id,
                    "taskType": provider_id,
                    "task": task,
                }),
                std::time::Duration::from_millis(500),
            )
            .await
            .map_err(|e| {
                if e.starts_with("RPC request 'tasks.resolveTask' timed out") {
                    "Timed out waiting for resolveTask".to_string()
                } else {
                    e
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_task_manager_lifecycle_and_events() {
        let mgr = ExtHostTaskManager::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_task_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        mgr.handle_rpc_call(
            "tasks.registerProvider",
            &json!({ "id": "task_cargo", "type": "cargo" }),
        )
        .unwrap();
        assert_eq!(mgr.list_providers().len(), 1);

        let exec_res = mgr
            .handle_rpc_call(
                "tasks.executeTask",
                &json!({
                    "id": "exec_1",
                    "task": {
                        "name": "cargo build",
                        "source": "cargo",
                        "definition": { "type": "cargo", "command": "build" }
                    }
                }),
            )
            .unwrap();
        assert_eq!(exec_res["executed"], true);
        assert_eq!(mgr.list_active_executions().len(), 1);

        mgr.handle_rpc_call(
            "tasks.reportTaskProcessStart",
            &json!({ "id": "exec_1", "processId": 4242 }),
        )
        .unwrap();
        assert_eq!(mgr.get_execution("exec_1").unwrap().process_id, Some(4242));

        mgr.handle_rpc_call(
            "tasks.reportTaskProcessEnd",
            &json!({ "id": "exec_1", "exitCode": 0 }),
        )
        .unwrap();
        assert_eq!(mgr.get_execution("exec_1").unwrap().exit_code, Some(0));
        assert!(mgr.list_active_executions().is_empty());

        let exec2 = mgr
            .handle_rpc_call(
                "tasks.executeTask",
                &json!({
                    "id": "exec_2",
                    "task": { "name": "watch", "source": "cargo" }
                }),
            )
            .unwrap();
        assert_eq!(exec2["id"], "exec_2");
        mgr.handle_rpc_call("tasks.terminateTask", &json!({ "id": "exec_2" }))
            .unwrap();
        assert_eq!(mgr.get_execution("exec_2").unwrap().status, "terminated");

        mgr.handle_rpc_call("tasks.unregisterProvider", &json!({ "id": "task_cargo" }))
            .unwrap();
        assert!(mgr.list_providers().is_empty());
        assert_eq!(events.lock().unwrap().len(), 7);
    }

    #[tokio::test]
    async fn test_task_request_roundtrip() {
        let mgr = ExtHostTaskManager::new();
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(
                1,
                Some(json!([{ "name": "cargo test", "source": "cargo" }])),
                None,
            );
        });

        let tasks = mgr.request_provide_tasks(&rpc, "cargo").await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["name"], "cargo test");
    }

    #[test]
    fn test_task_terminate_rejects_exited_and_already_terminated() {
        let mgr = ExtHostTaskManager::new();
        mgr.handle_rpc_call(
            "tasks.executeTask",
            &json!({ "id": "ex1", "task": { "name": "build", "source": "cargo" } }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tasks.reportTaskProcessEnd",
            &json!({ "id": "ex1", "exitCode": 0 }),
        )
        .unwrap();
        let _ = mgr.take_pending_events();

        // Terminating an already-exited task must error without overwriting status or emitting.
        assert!(mgr
            .handle_rpc_call("tasks.terminateTask", &json!({ "id": "ex1" }))
            .is_err());
        assert_eq!(mgr.get_execution("ex1").unwrap().status, "exited");
        assert!(mgr.take_pending_events().is_empty());

        mgr.handle_rpc_call(
            "tasks.executeTask",
            &json!({ "id": "ex2", "task": { "name": "watch", "source": "cargo" } }),
        )
        .unwrap();
        mgr.handle_rpc_call("tasks.terminateTask", &json!({ "id": "ex2" }))
            .unwrap();
        let _ = mgr.take_pending_events();

        // Terminating an already-terminated task must error without duplicate TaskTerminated.
        assert!(mgr
            .handle_rpc_call("tasks.terminateTask", &json!({ "id": "ex2" }))
            .is_err());
        assert!(mgr.take_pending_events().is_empty());
    }
}
