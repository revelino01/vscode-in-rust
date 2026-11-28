//! Extension Host activation, RPC latency, and memory profiler (`Developer: Show Running Extensions`).

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionActivationProfile {
    pub extension_id: String,
    pub activation_event: String,
    pub duration_ms: u64,
    pub succeeded: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcMethodMetrics {
    pub method: String,
    pub call_count: u64,
    pub error_count: u64,
    pub total_us: u64,
    pub max_us: u64,
}

impl RpcMethodMetrics {
    pub fn avg_us(&self) -> u64 {
        self.total_us.checked_div(self.call_count).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtHostMemoryStats {
    pub rss_bytes: u64,
    pub heap_total_bytes: u64,
    pub heap_used_bytes: u64,
    pub external_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtHostProfileSnapshot {
    pub activations: Vec<ExtensionActivationProfile>,
    pub rpc_methods: Vec<RpcMethodMetrics>,
    pub memory: Option<ExtHostMemoryStats>,
    pub total_rpc_calls: u64,
    pub total_rpc_errors: u64,
}

#[derive(Clone, Default)]
pub struct ExtHostProfiler {
    inner: Arc<RwLock<ProfilerInner>>,
}

#[derive(Default)]
struct ProfilerInner {
    in_flight_activations: HashMap<String, (Instant, String)>,
    activations: HashMap<String, ExtensionActivationProfile>,
    rpc_methods: HashMap<String, RpcMethodMetrics>,
    memory: Option<ExtHostMemoryStats>,
}

impl ExtHostProfiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn begin_activation(&self, extension_id: &str, activation_event: &str) {
        self.inner.write().in_flight_activations.insert(
            extension_id.to_string(),
            (Instant::now(), activation_event.to_string()),
        );
    }

    pub fn finish_activation(
        &self,
        extension_id: &str,
        succeeded: bool,
        error: Option<String>,
    ) -> ExtensionActivationProfile {
        let mut inner = self.inner.write();
        let (elapsed, activation_event) = inner
            .in_flight_activations
            .remove(extension_id)
            .map(|(t, ev)| (t.elapsed(), ev))
            .unwrap_or_else(|| (Duration::ZERO, "*".to_string()));
        let rec = ExtensionActivationProfile {
            extension_id: extension_id.to_string(),
            activation_event,
            duration_ms: elapsed.as_millis() as u64,
            succeeded,
            error,
        };
        inner.activations.insert(extension_id.to_string(), rec.clone());
        rec
    }

    pub fn record_activation_duration(
        &self,
        extension_id: &str,
        activation_event: &str,
        duration: Duration,
        succeeded: bool,
        error: Option<String>,
    ) {
        self.inner.write().activations.insert(
            extension_id.to_string(),
            ExtensionActivationProfile {
                extension_id: extension_id.to_string(),
                activation_event: activation_event.to_string(),
                duration_ms: duration.as_millis() as u64,
                succeeded,
                error,
            },
        );
    }

    pub fn record_rpc_call(&self, method: &str, duration: Duration, succeeded: bool) {
        let us = duration.as_micros().max(1) as u64;
        let mut inner = self.inner.write();
        let entry = inner
            .rpc_methods
            .entry(method.to_string())
            .or_insert_with(|| RpcMethodMetrics {
                method: method.to_string(),
                call_count: 0,
                error_count: 0,
                total_us: 0,
                max_us: 0,
            });
        entry.call_count += 1;
        if !succeeded {
            entry.error_count += 1;
        }
        entry.total_us = entry.total_us.saturating_add(us);
        entry.max_us = entry.max_us.max(us);
    }

    pub fn update_memory(&self, stats: ExtHostMemoryStats) {
        self.inner.write().memory = Some(stats);
    }

    pub fn snapshot(&self) -> ExtHostProfileSnapshot {
        let inner = self.inner.read();
        let mut activations: Vec<ExtensionActivationProfile> =
            inner.activations.values().cloned().collect();
        activations.sort_by(|a, b| {
            b.duration_ms
                .cmp(&a.duration_ms)
                .then_with(|| a.extension_id.cmp(&b.extension_id))
        });

        let mut rpc_methods: Vec<RpcMethodMetrics> =
            inner.rpc_methods.values().cloned().collect();
        rpc_methods.sort_by(|a, b| {
            b.total_us
                .cmp(&a.total_us)
                .then_with(|| a.method.cmp(&b.method))
        });

        let total_rpc_calls = rpc_methods.iter().map(|m| m.call_count).sum();
        let total_rpc_errors = rpc_methods.iter().map(|m| m.error_count).sum();

        ExtHostProfileSnapshot {
            activations,
            rpc_methods,
            memory: inner.memory.clone(),
            total_rpc_calls,
            total_rpc_errors,
        }
    }

    pub fn format_running_extensions_report(&self) -> String {
        let snap = self.snapshot();
        let mut lines = vec!["=== Running Extensions Profile ===".to_string()];
        if snap.activations.is_empty() {
            lines.push("No extension activations recorded yet.".to_string());
        } else {
            for act in &snap.activations {
                let status = if act.succeeded { "active" } else { "failed" };
                lines.push(format!(
                    "- {} [{}] {}ms (event: {})",
                    act.extension_id, status, act.duration_ms, act.activation_event
                ));
            }
        }
        if let Some(mem) = &snap.memory {
            lines.push(format!(
                "Memory: RSS {} MB | Heap Used {} MB / {} MB",
                mem.rss_bytes / (1024 * 1024),
                mem.heap_used_bytes / (1024 * 1024),
                mem.heap_total_bytes / (1024 * 1024)
            ));
        }
        lines.push(format!(
            "RPC Summary: {} calls ({} errors) across {} methods",
            snap.total_rpc_calls,
            snap.total_rpc_errors,
            snap.rpc_methods.len()
        ));
        for m in snap.rpc_methods.iter().take(10) {
            lines.push(format!(
                "  {} -> {} calls, avg {}us, max {}us ({} err)",
                m.method,
                m.call_count,
                m.avg_us(),
                m.max_us,
                m.error_count
            ));
        }
        lines.join("\n")
    }

    pub fn clear(&self) {
        let mut inner = self.inner.write();
        inner.in_flight_activations.clear();
        inner.activations.clear();
        inner.rpc_methods.clear();
        inner.memory = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_host_profiler_records_activations_and_rpcs() {
        let profiler = ExtHostProfiler::new();
        profiler.begin_activation("esbenp.prettier-vscode", "onLanguage:typescript");
        std::thread::sleep(Duration::from_millis(5));
        let act = profiler.finish_activation("esbenp.prettier-vscode", true, None);
        assert!(act.succeeded);
        profiler.record_activation_duration(
            "dbaeumer.vscode-eslint",
            "*",
            Duration::from_millis(42),
            true,
            None,
        );
        profiler.record_rpc_call("languages.hover", Duration::from_micros(320), true);
        profiler.record_rpc_call("languages.hover", Duration::from_micros(680), false);
        profiler.update_memory(ExtHostMemoryStats {
            rss_bytes: 64 * 1024 * 1024,
            heap_total_bytes: 32 * 1024 * 1024,
            heap_used_bytes: 18 * 1024 * 1024,
            external_bytes: 2 * 1024 * 1024,
        });

        let snap = profiler.snapshot();
        assert_eq!(snap.activations[0].extension_id, "dbaeumer.vscode-eslint");
        assert_eq!(snap.total_rpc_calls, 2);
        assert_eq!(snap.total_rpc_errors, 1);
        assert_eq!(snap.rpc_methods[0].max_us, 680);
        let report = profiler.format_running_extensions_report();
        assert!(report.contains("dbaeumer.vscode-eslint"));
        assert!(report.contains("Memory: RSS 64 MB"));
    }
}
