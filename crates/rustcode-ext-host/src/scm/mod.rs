//! Extension Host Source Control Management (`vscode.scm.*`).

use crate::rpc::RpcBridge;
use parking_lot::RwLock;
use rustcode_protocols::git::{GitStatusEntry, GitStatusKind, ScmRepository};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtScmInputBoxState {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub validation_message: Option<Value>,
}

fn default_true() -> bool {
    true
}

impl Default for ExtScmInputBoxState {
    fn default() -> Self {
        Self {
            value: String::new(),
            placeholder: String::new(),
            enabled: true,
            visible: true,
            validation_message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtScmResourceState {
    pub resource_uri: String,
    #[serde(default)]
    pub command: Option<Value>,
    #[serde(default)]
    pub decorations: Option<Value>,
    #[serde(default)]
    pub context_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtScmResourceGroupState {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub hide_when_empty: bool,
    #[serde(default)]
    pub resource_states: Vec<ExtScmResourceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtSourceControlState {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub root_uri: Option<String>,
    #[serde(default)]
    pub input_box: ExtScmInputBoxState,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub commit_template: Option<String>,
    #[serde(default)]
    pub accept_input_command: Option<Value>,
    #[serde(default)]
    pub status_bar_commands: Vec<Value>,
    #[serde(default)]
    pub has_quick_diff_provider: bool,
    #[serde(default)]
    pub groups: Vec<ExtScmResourceGroupState>,
    #[serde(default)]
    pub is_disposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtScmEvent {
    SourceControlCreated {
        id: String,
        label: String,
        root_uri: Option<String>,
    },
    SourceControlUpdated {
        id: String,
        count: Option<u32>,
    },
    SourceControlDisposed {
        id: String,
    },
    ResourceGroupCreated {
        source_control_id: String,
        group_id: String,
        label: String,
    },
    ResourceGroupUpdated {
        source_control_id: String,
        group_id: String,
        resource_count: usize,
    },
    ResourceGroupDisposed {
        source_control_id: String,
        group_id: String,
    },
    InputBoxUpdated {
        source_control_id: String,
        value: String,
    },
}

pub type ScmEventListener = Arc<dyn Fn(&ExtScmEvent) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ExtHostScmManager {
    source_controls: Arc<RwLock<HashMap<String, ExtSourceControlState>>>,
    order: Arc<RwLock<Vec<String>>>,
    listeners: Arc<RwLock<Vec<ScmEventListener>>>,
    pending_events: Arc<RwLock<Vec<ExtScmEvent>>>,
    revision: Arc<AtomicU64>,
}

pub fn parse_git_status_kind(hint: &str) -> GitStatusKind {
    let lower = hint.trim().to_ascii_lowercase();
    match lower.as_str() {
        "u" | "untracked" => GitStatusKind::Untracked,
        "a" | "added" => GitStatusKind::Added,
        "d" | "deleted" => GitStatusKind::Deleted,
        "r" | "renamed" => GitStatusKind::Renamed,
        "c" | "conflicted" | "conflict" => GitStatusKind::Conflicted,
        _ => GitStatusKind::Modified,
    }
}

pub fn git_status_badge(kind: GitStatusKind) -> &'static str {
    kind.badge_str()
}

fn strip_file_scheme(uri_or_path: &str) -> PathBuf {
    let raw = uri_or_path
        .strip_prefix("file://")
        .unwrap_or(uri_or_path);
    PathBuf::from(raw)
}

impl ExtHostScmManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn take_pending_events(&self) -> Vec<ExtScmEvent> {
        std::mem::take(&mut *self.pending_events.write())
    }

    pub fn on_scm_event<F>(&self, listener: F)
    where
        F: Fn(&ExtScmEvent) + Send + Sync + 'static,
    {
        self.listeners.write().push(Arc::new(listener));
    }

    fn emit(&self, event: ExtScmEvent) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.pending_events.write().push(event.clone());
        let listeners = self.listeners.read().clone();
        for cb in listeners {
            cb(&event);
        }
    }

    pub fn get_source_control(&self, id: &str) -> Option<ExtSourceControlState> {
        self.source_controls.read().get(id).cloned()
    }

    pub fn list_source_controls(&self) -> Vec<ExtSourceControlState> {
        let map = self.source_controls.read();
        let order = self.order.read();
        order
            .iter()
            .filter_map(|id| map.get(id))
            .filter(|sc| !sc.is_disposed)
            .cloned()
            .collect()
    }

    pub fn to_git_status_entries(&self, sc_id: &str) -> Vec<GitStatusEntry> {
        let Some(sc) = self.get_source_control(sc_id) else {
            return Vec::new();
        };
        if sc.is_disposed {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for group in &sc.groups {
            let group_id_lower = group.id.to_ascii_lowercase();
            let group_label_lower = group.label.to_ascii_lowercase();
            let label_mentions_unstaged = group_label_lower.contains("unstaged")
                || group_label_lower.contains("not staged")
                || group_label_lower.contains("not-staged");
            let label_mentions_staged = !label_mentions_unstaged
                && group_label_lower
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|word| word == "staged");
            let is_staged =
                group_id_lower == "index" || group_id_lower == "staged" || label_mentions_staged;
            let is_untracked =
                group_id_lower == "untracked" || group_label_lower.contains("untracked");

            for res in &group.resource_states {
                let status_hint = res
                    .decorations
                    .as_ref()
                    .and_then(|d| {
                        d.get("tooltip")
                            .or_else(|| d.get("letter"))
                            .or_else(|| d.get("status"))
                            .and_then(|v| v.as_str())
                    })
                    .or(res.context_value.as_deref())
                    .unwrap_or(if is_untracked { "untracked" } else { "modified" });
                let status = parse_git_status_kind(status_hint);
                entries.push(GitStatusEntry {
                    path: strip_file_scheme(&res.resource_uri),
                    status,
                    is_staged,
                });
            }
        }
        entries
    }

    pub fn to_scm_repository(&self, sc_id: &str) -> Option<ScmRepository> {
        let sc = self.get_source_control(sc_id)?;
        if sc.is_disposed {
            return None;
        }
        let root = sc
            .root_uri
            .as_deref()
            .map(strip_file_scheme)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut repo = ScmRepository::new(root);
        for entry in self.to_git_status_entries(sc_id) {
            if entry.is_staged {
                repo.staged.push(entry);
            } else if entry.status == GitStatusKind::Untracked {
                repo.untracked.push(entry);
            } else {
                repo.changes.push(entry);
            }
        }
        Some(repo)
    }

    pub fn handle_rpc_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "scm.createSourceControl" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing source control id".to_string())?
                    .to_string();
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let root_uri = params
                    .get("rootUri")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let state = ExtSourceControlState {
                    id: id.clone(),
                    label: label.clone(),
                    root_uri: root_uri.clone(),
                    input_box: ExtScmInputBoxState::default(),
                    count: None,
                    commit_template: None,
                    accept_input_command: None,
                    status_bar_commands: Vec::new(),
                    has_quick_diff_provider: false,
                    groups: Vec::new(),
                    is_disposed: false,
                };

                {
                    let mut map = self.source_controls.write();
                    let mut order = self.order.write();
                    if !map.contains_key(&id) {
                        order.push(id.clone());
                    }
                    map.insert(id.clone(), state);
                }

                self.emit(ExtScmEvent::SourceControlCreated {
                    id: id.clone(),
                    label,
                    root_uri,
                });
                Ok(json!({ "created": true, "id": id }))
            }
            "scm.updateSourceControl" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing source control id".to_string())?;
                let count = {
                    let mut map = self.source_controls.write();
                    let sc = map
                        .get_mut(id)
                        .ok_or_else(|| format!("Source control '{}' not found", id))?;
                    if sc.is_disposed {
                        return Err(format!("Source control '{}' is disposed", id));
                    }
                    if let Some(c) = params.get("count").and_then(|v| v.as_u64()) {
                        sc.count = Some(c as u32);
                    }
                    if let Some(tpl) = params.get("commitTemplate").and_then(|v| v.as_str()) {
                        sc.commit_template = Some(tpl.to_string());
                    }
                    if let Some(cmd) = params.get("acceptInputCommand") {
                        if !cmd.is_null() {
                            sc.accept_input_command = Some(cmd.clone());
                        }
                    }
                    if let Some(cmds) = params
                        .get("statusBarCommands")
                        .and_then(|v| v.as_array())
                    {
                        sc.status_bar_commands = cmds.clone();
                    }
                    if let Some(has_qd) = params.get("hasQuickDiffProvider").and_then(|v| v.as_bool())
                    {
                        sc.has_quick_diff_provider = has_qd;
                    }
                    sc.count
                };
                self.emit(ExtScmEvent::SourceControlUpdated {
                    id: id.to_string(),
                    count,
                });
                Ok(json!({ "updated": true, "id": id }))
            }
            "scm.disposeSourceControl" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing source control id".to_string())?;
                let disposed = {
                    let mut map = self.source_controls.write();
                    if let Some(sc) = map.get_mut(id) {
                        if sc.is_disposed {
                            false
                        } else {
                            sc.is_disposed = true;
                            true
                        }
                    } else {
                        false
                    }
                };
                if disposed {
                    self.emit(ExtScmEvent::SourceControlDisposed { id: id.to_string() });
                }
                Ok(json!({ "disposed": disposed, "id": id }))
            }
            "scm.createResourceGroup" => {
                let sc_id = params
                    .get("sourceControlId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing sourceControlId".to_string())?;
                let group_id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing resource group id".to_string())?
                    .to_string();
                let label = params
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&group_id)
                    .to_string();
                let hide_when_empty = params
                    .get("hideWhenEmpty")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                {
                    let mut map = self.source_controls.write();
                    let sc = map
                        .get_mut(sc_id)
                        .ok_or_else(|| format!("Source control '{}' not found", sc_id))?;
                    if sc.is_disposed {
                        return Err(format!("Source control '{}' is disposed", sc_id));
                    }
                    if let Some(existing) = sc.groups.iter_mut().find(|g| g.id == group_id) {
                        existing.label = label.clone();
                        existing.hide_when_empty = hide_when_empty;
                    } else {
                        sc.groups.push(ExtScmResourceGroupState {
                            id: group_id.clone(),
                            label: label.clone(),
                            hide_when_empty,
                            resource_states: Vec::new(),
                        });
                    }
                }

                self.emit(ExtScmEvent::ResourceGroupCreated {
                    source_control_id: sc_id.to_string(),
                    group_id: group_id.clone(),
                    label,
                });
                Ok(json!({ "created": true, "id": group_id }))
            }
            "scm.updateResourceGroup" => {
                let sc_id = params
                    .get("sourceControlId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing sourceControlId".to_string())?;
                let group_id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing resource group id".to_string())?;
                let parsed_states: Option<Vec<ExtScmResourceState>> =
                    match params.get("resourceStates") {
                        Some(states) if !states.is_null() => Some(
                            serde_json::from_value(states.clone())
                                .map_err(|e| format!("Invalid resourceStates: {}", e))?,
                        ),
                        _ => None,
                    };

                let count = {
                    let mut map = self.source_controls.write();
                    let sc = map
                        .get_mut(sc_id)
                        .ok_or_else(|| format!("Source control '{}' not found", sc_id))?;
                    if sc.is_disposed {
                        return Err(format!("Source control '{}' is disposed", sc_id));
                    }
                    let group = sc
                        .groups
                        .iter_mut()
                        .find(|g| g.id == group_id)
                        .ok_or_else(|| format!("Resource group '{}' not found", group_id))?;
                    if let Some(label) = params.get("label").and_then(|v| v.as_str()) {
                        group.label = label.to_string();
                    }
                    if let Some(hide) = params.get("hideWhenEmpty").and_then(|v| v.as_bool()) {
                        group.hide_when_empty = hide;
                    }
                    if let Some(parsed) = parsed_states {
                        group.resource_states = parsed;
                    }
                    group.resource_states.len()
                };

                self.emit(ExtScmEvent::ResourceGroupUpdated {
                    source_control_id: sc_id.to_string(),
                    group_id: group_id.to_string(),
                    resource_count: count,
                });
                Ok(json!({ "updated": true, "id": group_id, "count": count }))
            }
            "scm.disposeResourceGroup" => {
                let sc_id = params
                    .get("sourceControlId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing sourceControlId".to_string())?;
                let group_id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing resource group id".to_string())?;

                let removed = {
                    let mut map = self.source_controls.write();
                    let sc = map
                        .get_mut(sc_id)
                        .ok_or_else(|| format!("Source control '{}' not found", sc_id))?;
                    if sc.is_disposed {
                        return Err(format!("Source control '{}' is disposed", sc_id));
                    }
                    let before = sc.groups.len();
                    sc.groups.retain(|g| g.id != group_id);
                    sc.groups.len() < before
                };

                if removed {
                    self.emit(ExtScmEvent::ResourceGroupDisposed {
                        source_control_id: sc_id.to_string(),
                        group_id: group_id.to_string(),
                    });
                }
                Ok(json!({ "disposed": removed, "id": group_id }))
            }
            "scm.updateInputBox" => {
                let sc_id = params
                    .get("sourceControlId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing sourceControlId".to_string())?;

                let value = {
                    let mut map = self.source_controls.write();
                    let sc = map
                        .get_mut(sc_id)
                        .ok_or_else(|| format!("Source control '{}' not found", sc_id))?;
                    if sc.is_disposed {
                        return Err(format!("Source control '{}' is disposed", sc_id));
                    }
                    if let Some(v) = params.get("value").and_then(|v| v.as_str()) {
                        sc.input_box.value = v.to_string();
                    }
                    if let Some(p) = params.get("placeholder").and_then(|v| v.as_str()) {
                        sc.input_box.placeholder = p.to_string();
                    }
                    if let Some(e) = params.get("enabled").and_then(|v| v.as_bool()) {
                        sc.input_box.enabled = e;
                    }
                    if let Some(vis) = params.get("visible").and_then(|v| v.as_bool()) {
                        sc.input_box.visible = vis;
                    }
                    if let Some(msg) = params.get("validationMessage") {
                        sc.input_box.validation_message =
                            if msg.is_null() { None } else { Some(msg.clone()) };
                    }
                    sc.input_box.value.clone()
                };

                self.emit(ExtScmEvent::InputBoxUpdated {
                    source_control_id: sc_id.to_string(),
                    value,
                });
                Ok(json!({ "updated": true, "sourceControlId": sc_id }))
            }
            _ => Err(format!("Unsupported SCM RPC method: {}", method)),
        }
    }

    pub async fn request_provide_original_resource(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        uri: &str,
    ) -> Result<Option<String>, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "scm.provideOriginalResource",
            json!({
                "providerId": provider_id,
                "sourceControlId": provider_id,
                "uri": uri,
            }),
        );

        let val = tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for provideOriginalResource".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))??;

        if val.is_null() {
            Ok(None)
        } else if let Some(s) = val.as_str() {
            Ok(Some(s.to_string()))
        } else if let Some(path) = val.get("path").and_then(|p| p.as_str()) {
            let scheme = val
                .get("scheme")
                .and_then(|s| s.as_str())
                .unwrap_or("file");
            Ok(Some(format!("{}://{}", scheme, path)))
        } else {
            Ok(Some(val.to_string()))
        }
    }

    pub async fn request_accept_input(
        &self,
        rpc_bridge: &RpcBridge,
        provider_id: &str,
        value: Option<&str>,
    ) -> Result<Value, String> {
        let (_req, rx) = rpc_bridge.create_request(
            "scm.acceptInput",
            json!({
                "providerId": provider_id,
                "sourceControlId": provider_id,
                "value": value,
            }),
        );

        tokio::time::timeout(std::time::Duration::from_millis(500), rx)
            .await
            .map_err(|_| "Timed out waiting for acceptInput".to_string())?
            .map_err(|e| format!("RPC channel closed: {}", e))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_scm_manager_lifecycle_and_git_mapping() {
        let mgr = ExtHostScmManager::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        mgr.on_scm_event(move |ev| {
            events_clone.lock().unwrap().push(ev.clone());
        });

        let rev0 = mgr.revision();
        mgr.handle_rpc_call(
            "scm.createSourceControl",
            &json!({ "id": "git", "label": "Git", "rootUri": "file:///workspace/demo" }),
        )
        .unwrap();
        assert!(mgr.revision() > rev0);

        mgr.handle_rpc_call(
            "scm.updateInputBox",
            &json!({ "sourceControlId": "git", "value": "feat: add scm", "placeholder": "Message" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.createResourceGroup",
            &json!({ "sourceControlId": "git", "id": "index", "label": "Staged Changes" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.createResourceGroup",
            &json!({ "sourceControlId": "git", "id": "workingTree", "label": "Changes" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.updateResourceGroup",
            &json!({
                "sourceControlId": "git",
                "id": "index",
                "resourceStates": [{
                    "resourceUri": "file:///workspace/demo/Cargo.toml",
                    "decorations": { "tooltip": "Added" }
                }]
            }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.updateResourceGroup",
            &json!({
                "sourceControlId": "git",
                "id": "workingTree",
                "resourceStates": [{
                    "resourceUri": "file:///workspace/demo/src/lib.rs",
                    "decorations": { "tooltip": "M" }
                }]
            }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.updateSourceControl",
            &json!({ "id": "git", "count": 2, "hasQuickDiffProvider": true }),
        )
        .unwrap();

        let repo = mgr.to_scm_repository("git").unwrap();
        assert_eq!(repo.root, PathBuf::from("/workspace/demo"));
        assert_eq!(repo.staged.len(), 1);
        assert_eq!(repo.staged[0].status, GitStatusKind::Added);
        assert_eq!(git_status_badge(repo.staged[0].status), "A");
        assert_eq!(repo.changes.len(), 1);
        assert_eq!(repo.changes[0].status, GitStatusKind::Modified);

        mgr.handle_rpc_call(
            "scm.disposeResourceGroup",
            &json!({ "sourceControlId": "git", "id": "workingTree" }),
        )
        .unwrap();
        assert_eq!(mgr.get_source_control("git").unwrap().groups.len(), 1);

        mgr.handle_rpc_call("scm.disposeSourceControl", &json!({ "id": "git" }))
            .unwrap();
        assert!(mgr.list_source_controls().is_empty());
        assert!(mgr
            .handle_rpc_call(
                "scm.updateInputBox",
                &json!({ "sourceControlId": "git", "value": "fail" }),
            )
            .is_err());
        assert_eq!(events.lock().unwrap().len(), 9);
        assert_eq!(mgr.take_pending_events().len(), 9);
    }

    #[tokio::test]
    async fn test_scm_request_provide_original_resource_roundtrip() {
        let mgr = ExtHostScmManager::new();
        let rpc = Arc::new(RpcBridge::new());
        let rpc_clone = rpc.clone();

        tokio::spawn(async move {
            tokio::task::yield_now().await;
            rpc_clone.handle_incoming_response(
                1,
                Some(json!("git:///workspace/main.rs")),
                None,
            );
        });

        let res = mgr
            .request_provide_original_resource(&rpc, "git", "file:///workspace/main.rs")
            .await
            .unwrap();
        assert_eq!(res.as_deref(), Some("git:///workspace/main.rs"));
    }

    #[test]
    fn test_scm_unstaged_changes_and_malformed_payload_guards() {
        let mgr = ExtHostScmManager::new();
        mgr.handle_rpc_call(
            "scm.createSourceControl",
            &json!({ "id": "git", "label": "Git", "rootUri": "file:///repo" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.createResourceGroup",
            &json!({ "sourceControlId": "git", "id": "workingTree", "label": "Unstaged Changes" }),
        )
        .unwrap();
        mgr.handle_rpc_call(
            "scm.updateResourceGroup",
            &json!({
                "sourceControlId": "git",
                "id": "workingTree",
                "resourceStates": [
                    { "resourceUri": "file:///repo/src/lib.rs", "decorations": { "tooltip": "Modified" } },
                    { "resourceUri": "untitled:Untitled-1", "decorations": { "tooltip": "Untracked" } }
                ]
            }),
        )
        .unwrap();

        let entries = mgr.to_git_status_entries("git");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| !e.is_staged));

        let repo = mgr.to_scm_repository("git").unwrap();
        assert!(repo.staged.is_empty());
        assert_eq!(repo.changes.len(), 1);
        assert_eq!(repo.untracked.len(), 1);

        // Malformed resourceStates must return Err and preserve existing states.
        assert!(mgr
            .handle_rpc_call(
                "scm.updateResourceGroup",
                &json!({
                    "sourceControlId": "git",
                    "id": "workingTree",
                    "resourceStates": "not-an-array"
                }),
            )
            .is_err());
        assert_eq!(
            mgr.get_source_control("git").unwrap().groups[0]
                .resource_states
                .len(),
            2
        );

        // Disposing resource group after disposing parent SourceControl must return Err.
        mgr.handle_rpc_call("scm.disposeSourceControl", &json!({ "id": "git" }))
            .unwrap();
        assert!(mgr
            .handle_rpc_call(
                "scm.disposeResourceGroup",
                &json!({ "sourceControlId": "git", "id": "workingTree" }),
            )
            .is_err());
    }
}
