//! Extension Host Testing Manager (`vscode.tests.*`).

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTestItemState {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub children: Vec<ExtTestItemState>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub can_resolve_children: bool,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTestRunProfileInfo {
    pub id: String,
    pub controller_id: String,
    pub label: String,
    pub kind: u8,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub supports_continuous_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTestResultRecord {
    pub test_id: String,
    pub state: String,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTestRunState {
    pub run_id: String,
    pub controller_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub persist: bool,
    #[serde(default)]
    pub is_ended: bool,
    #[serde(default)]
    pub results: HashMap<String, ExtTestResultRecord>,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtTestControllerState {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub items: Vec<ExtTestItemState>,
    #[serde(default)]
    pub profiles: HashMap<String, ExtTestRunProfileInfo>,
    #[serde(default)]
    pub is_disposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtTestEvent {
    ControllerCreated {
        id: String,
        label: String,
    },
    ControllerDisposed {
        id: String,
    },
    ItemsUpdated {
        controller_id: String,
        count: usize,
    },
    ProfileCreated {
        controller_id: String,
        profile_id: String,
        label: String,
    },
    ProfileDisposed {
        controller_id: String,
        profile_id: String,
    },
    RunCreated {
        run_id: String,
        controller_id: String,
    },
    TestStateUpdated {
        run_id: String,
        test_id: String,
        state: String,
    },
    RunOutputAppended {
        run_id: String,
        output: String,
    },
    RunEnded {
        run_id: String,
    },
    ResultsInvalidated {
        controller_id: String,
    },
}

pub type TestEventListener = Arc<dyn Fn(&ExtTestEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostTestManager {
    controllers: Arc<RwLock<HashMap<String, ExtTestControllerState>>>,
    runs: Arc<RwLock<HashMap<String, ExtTestRunState>>>,
    listeners: Arc<RwLock<Vec<TestEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtTestEvent>>>,
    revision: Arc<AtomicU64>,
}

fn update_children_recursive(
    items: &mut [ExtTestItemState],
    parent_id: &str,
    new_children: &[ExtTestItemState],
) -> bool {
    for item in items.iter_mut() {
        if item.id == parent_id {
            item.children = new_children.to_vec();
            return true;
        }
        if update_children_recursive(&mut item.children, parent_id, new_children) {
            return true;
        }
    }
    false
}

impl ExtHostTestManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtTestEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_test_event<F>(&self, listener: F)
    where
        F: Fn(&ExtTestEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtTestEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&event);
        }
    }

    pub fn get_controller(&self, id: &str) -> Option<ExtTestControllerState> {
        self.controllers.read().get(id).cloned()
    }

    pub fn list_controllers(&self) -> Vec<ExtTestControllerState> {
        self.controllers
            .read()
            .values()
            .filter(|c| !c.is_disposed)
            .cloned()
            .collect()
    }

    pub fn get_run(&self, run_id: &str) -> Option<ExtTestRunState> {
        self.runs.read().get(run_id).cloned()
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "tests.createController" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing test controller id".to_string())?
                    .to_string();
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let ctrl = ExtTestControllerState {
                    id: id.clone(),
                    label: label.clone(),
                    items: Vec::new(),
                    profiles: HashMap::new(),
                    is_disposed: false,
                };
                self.controllers.write().insert(id.clone(), ctrl);
                self.emit(ExtTestEvent::ControllerCreated {
                    id: id.clone(),
                    label,
                });
                Ok(json!({ "created": true, "id": id }))
            }
            "tests.disposeController" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing test controller id".to_string())?;
                let disposed = {
                    let mut guard = self.controllers.write();
                    if let Some(ctrl) = guard.get_mut(id) {
                        if ctrl.is_disposed {
                            false
                        } else {
                            ctrl.is_disposed = true;
                            true
                        }
                    } else {
                        false
                    }
                };
                if disposed {
                    self.emit(ExtTestEvent::ControllerDisposed { id: id.to_string() });
                }
                Ok(json!({ "disposed": disposed, "id": id }))
            }
            "tests.updateItems" => {
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controllerId".to_string())?;
                let parent_id = params.get("parentId").and_then(|v| v.as_str());
                let items: Vec<ExtTestItemState> = match params.get("items") {
                    Some(v) if !v.is_null() => serde_json::from_value(v.clone())
                        .map_err(|e| format!("Invalid test items: {}", e))?,
                    _ => Vec::new(),
                };

                {
                    let mut guard = self.controllers.write();
                    let ctrl = guard
                        .get_mut(controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                    if let Some(pid) = parent_id {
                        if !update_children_recursive(&mut ctrl.items, pid, &items) {
                            return Err(format!("Parent test item '{}' not found", pid));
                        }
                    } else {
                        ctrl.items = items.clone();
                    }
                }

                self.emit(ExtTestEvent::ItemsUpdated {
                    controller_id: controller_id.to_string(),
                    count: items.len(),
                });
                Ok(json!({ "updated": true, "count": items.len() }))
            }
            "tests.createRunProfile" => {
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controllerId".to_string())?;
                let profile_id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing profile id".to_string())?
                    .to_string();
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&profile_id)
                    .to_string();
                let kind = params.get("kind").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
                let is_default = params
                    .get("isDefault")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let tag = params
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let supports_continuous_run = params
                    .get("supportsContinuousRun")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let profile = ExtTestRunProfileInfo {
                    id: profile_id.clone(),
                    controller_id: controller_id.to_string(),
                    label: label.clone(),
                    kind,
                    is_default,
                    tag,
                    supports_continuous_run,
                };

                {
                    let mut guard = self.controllers.write();
                    let ctrl = guard
                        .get_mut(controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                    ctrl.profiles.insert(profile_id.clone(), profile);
                }

                self.emit(ExtTestEvent::ProfileCreated {
                    controller_id: controller_id.to_string(),
                    profile_id: profile_id.clone(),
                    label,
                });
                Ok(json!({ "created": true, "id": profile_id }))
            }
            "tests.disposeRunProfile" => {
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controllerId".to_string())?;
                let profile_id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing profile id".to_string())?;

                let removed = {
                    let mut guard = self.controllers.write();
                    let ctrl = guard
                        .get_mut(controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                    ctrl.profiles.remove(profile_id).is_some()
                };

                if removed {
                    self.emit(ExtTestEvent::ProfileDisposed {
                        controller_id: controller_id.to_string(),
                        profile_id: profile_id.to_string(),
                    });
                }
                Ok(json!({ "disposed": removed, "id": profile_id }))
            }
            "tests.createTestRun" => {
                let run_id = params
                    .get("runId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing runId".to_string())?
                    .to_string();
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controllerId".to_string())?
                    .to_string();
                {
                    let guard = self.controllers.read();
                    let ctrl = guard
                        .get(&controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                }
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let persist = params
                    .get("persist")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let run = ExtTestRunState {
                    run_id: run_id.clone(),
                    controller_id: controller_id.clone(),
                    name,
                    persist,
                    is_ended: false,
                    results: HashMap::new(),
                    output: String::new(),
                };
                self.runs.write().insert(run_id.clone(), run);
                self.emit(ExtTestEvent::RunCreated {
                    run_id: run_id.clone(),
                    controller_id,
                });
                Ok(json!({ "created": true, "runId": run_id }))
            }
            "tests.updateTestState" => {
                let run_id = params
                    .get("runId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing runId".to_string())?;
                let test_id = params
                    .get("testId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing testId".to_string())?
                    .to_string();
                let state = params
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("started")
                    .to_string();
                let duration = params.get("duration").and_then(|v| v.as_u64());
                let messages = match params.get("messages") {
                    Some(v) if !v.is_null() => v
                        .as_array()
                        .ok_or_else(|| "Invalid test messages: expected an array".to_string())?
                        .clone(),
                    _ => Vec::new(),
                };

                {
                    let mut runs = self.runs.write();
                    let run = runs
                        .get_mut(run_id)
                        .ok_or_else(|| format!("TestRun '{}' not found", run_id))?;
                    if run.is_ended {
                        return Err(format!("TestRun '{}' has already ended", run_id));
                    }
                    run.results.insert(
                        test_id.clone(),
                        ExtTestResultRecord {
                            test_id: test_id.clone(),
                            state: state.clone(),
                            duration,
                            messages,
                        },
                    );
                }

                self.emit(ExtTestEvent::TestStateUpdated {
                    run_id: run_id.to_string(),
                    test_id,
                    state,
                });
                Ok(json!({ "updated": true }))
            }
            "tests.appendRunOutput" => {
                let run_id = params
                    .get("runId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing runId".to_string())?;
                let output = params
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                {
                    let mut runs = self.runs.write();
                    let run = runs
                        .get_mut(run_id)
                        .ok_or_else(|| format!("TestRun '{}' not found", run_id))?;
                    if run.is_ended {
                        return Err(format!("TestRun '{}' has already ended", run_id));
                    }
                    run.output.push_str(&output);
                }
                self.emit(ExtTestEvent::RunOutputAppended {
                    run_id: run_id.to_string(),
                    output,
                });
                Ok(json!({ "appended": true }))
            }
            "tests.endTestRun" => {
                let run_id = params
                    .get("runId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing runId".to_string())?;
                let controller_id = {
                    let runs = self.runs.read();
                    let run = runs
                        .get(run_id)
                        .ok_or_else(|| format!("TestRun '{}' not found", run_id))?;
                    if run.is_ended {
                        return Err(format!("TestRun '{}' has already ended", run_id));
                    }
                    run.controller_id.clone()
                };
                {
                    let controllers = self.controllers.read();
                    let ctrl = controllers
                        .get(&controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                }
                {
                    let mut runs = self.runs.write();
                    let run = runs
                        .get_mut(run_id)
                        .ok_or_else(|| format!("TestRun '{}' not found", run_id))?;
                    if run.is_ended {
                        return Err(format!("TestRun '{}' has already ended", run_id));
                    }
                    run.is_ended = true;
                }
                self.emit(ExtTestEvent::RunEnded {
                    run_id: run_id.to_string(),
                });
                Ok(json!({ "ended": true, "runId": run_id }))
            }
            "tests.invalidateTestResults" => {
                let controller_id = params
                    .get("controllerId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing controllerId".to_string())?;
                {
                    let controllers = self.controllers.read();
                    let ctrl = controllers
                        .get(controller_id)
                        .ok_or_else(|| format!("TestController '{}' not found", controller_id))?;
                    if ctrl.is_disposed {
                        return Err(format!("TestController '{}' is disposed", controller_id));
                    }
                }
                self.emit(ExtTestEvent::ResultsInvalidated {
                    controller_id: controller_id.to_string(),
                });
                Ok(json!({ "invalidated": true }))
            }
            _ => Err(format!("Unsupported test RPC method: {}", method)),
        }
    }

    pub async fn request_resolve_children(
        &self,
        rpc_bridge: &RpcBridge,
        controller_id: &str,
        item_id: Option<&str>,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "tests.resolveChildren",
            json!({
                "controllerId": controller_id,
                "providerId": controller_id,
                "itemId": item_id,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for resolveChildren".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }

    pub async fn request_refresh_tests(
        &self,
        rpc_bridge: &RpcBridge,
        controller_id: &str,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "tests.refreshTests",
            json!({
                "controllerId": controller_id,
                "providerId": controller_id,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for refreshTests".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }

    pub async fn request_run_tests(
        &self,
        rpc_bridge: &RpcBridge,
        controller_id: &str,
        request: Value,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "tests.runTests",
            json!({
                "controllerId": controller_id,
                "providerId": controller_id,
                "request": request,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for runTests".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_testing_manager_lifecycle() {
        let mgr = ExtHostTestManager::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_test_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        mgr.handle_rpc_call(
            "tests.createController",
            &json!({ "id": "rust-tests", "label": "Rust Tests" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateItems",
            &json!({
                "controllerId": "rust-tests",
                "items": [{ "id": "t1", "label": "test_a" }]
            }),
        )
        .unwrap();
        assert_eq!(mgr.get_controller("rust-tests").unwrap().items.len(), 1);

        mgr.handle_rpc_call(
            "tests.createRunProfile",
            &json!({
                "controllerId": "rust-tests",
                "id": "prof_1",
                "label": "Run",
                "kind": 1,
                "isDefault": true
            }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.createTestRun",
            &json!({ "runId": "run_1", "controllerId": "rust-tests", "name": "CI" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateTestState",
            &json!({ "runId": "run_1", "testId": "t1", "state": "passed", "duration": 15 }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.appendRunOutput",
            &json!({ "runId": "run_1", "output": "ok\n" }),
        )
        .unwrap();
        mgr.handle_rpc_call("tests.endTestRun", &json!({ "runId": "run_1" }))
            .unwrap();

        let run = mgr.get_run("run_1").unwrap();
        assert!(run.is_ended);
        assert_eq!(run.results["t1"].state, "passed");
        assert_eq!(run.output, "ok\n");

        mgr.handle_rpc_call("tests.disposeController", &json!({ "id": "rust-tests" }))
            .unwrap();
        assert!(mgr.list_controllers().is_empty());
        assert!(mgr
            .handle_rpc_call(
                "tests.updateItems",
                &json!({ "controllerId": "rust-tests", "items": [] }),
            )
            .is_err());
        assert_eq!(events.lock().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn test_testing_request_roundtrip() {
        let mgr = ExtHostTestManager::new();
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(1, Some(json!({ "ran": true })), None);
        });

        let res = mgr
            .request_run_tests(&rpc, "rust-tests", json!({ "profileId": "prof_1" }))
            .await
            .unwrap();
        assert_eq!(res["ran"], true);
    }

    #[test]
    fn test_testing_malformed_items_and_post_lifecycle_guards() {
        let mgr = ExtHostTestManager::new();
        mgr.handle_rpc_call(
            "tests.createController",
            &json!({ "id": "c1", "label": "C1" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.updateItems",
            &json!({ "controllerId": "c1", "items": [{ "id": "t1", "label": "T1" }] }),
        )
        .unwrap();

        // Malformed items must return Err and preserve existing items.
        assert!(mgr
            .handle_rpc_call(
                "tests.updateItems",
                &json!({ "controllerId": "c1", "items": "not-an-array" }),
            )
            .is_err());
        assert_eq!(mgr.get_controller("c1").unwrap().items.len(), 1);

        mgr.handle_rpc_call(
            "tests.createRunProfile",
            &json!({ "controllerId": "c1", "id": "p1", "label": "Run", "kind": 1 }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "tests.createTestRun",
            &json!({ "runId": "r1", "controllerId": "c1" }),
        )
        .unwrap();
        assert!(mgr
            .handle_rpc_call(
                "tests.updateTestState",
                &json!({ "runId": "r1", "testId": "t1", "state": "failed", "messages": "bad" }),
            )
            .is_err());
        mgr.handle_rpc_call("tests.endTestRun", &json!({ "runId": "r1" }))
            .unwrap();
        let _ = mgr.take_pending_events();

        // Ending an already-ended run must return Err and not emit duplicate RunEnded.
        assert!(mgr
            .handle_rpc_call("tests.endTestRun", &json!({ "runId": "r1" }))
            .is_err());
        assert!(mgr.take_pending_events().is_empty());

        // Missing controller guards.
        assert!(mgr
            .handle_rpc_call(
                "tests.invalidateTestResults",
                &json!({ "controllerId": "missing" }),
            )
            .is_err());
        assert!(mgr
            .handle_rpc_call(
                "tests.disposeRunProfile",
                &json!({ "controllerId": "missing", "id": "p1" }),
            )
            .is_err());

        // Post-dispose controller guards.
        mgr.handle_rpc_call(
            "tests.createTestRun",
            &json!({ "runId": "r2", "controllerId": "c1" }),
        )
        .unwrap();
        mgr.handle_rpc_call("tests.disposeController", &json!({ "id": "c1" }))
            .unwrap();
        let _ = mgr.take_pending_events();

        assert!(mgr
            .handle_rpc_call(
                "tests.disposeRunProfile",
                &json!({ "controllerId": "c1", "id": "p1" }),
            )
            .is_err());
        assert!(mgr
            .handle_rpc_call(
                "tests.invalidateTestResults",
                &json!({ "controllerId": "c1" }),
            )
            .is_err());
        assert!(mgr
            .handle_rpc_call("tests.endTestRun", &json!({ "runId": "r2" }))
            .is_err());
        assert!(mgr.take_pending_events().is_empty());
    }
}
