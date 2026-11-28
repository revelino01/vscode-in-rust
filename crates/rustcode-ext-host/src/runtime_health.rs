//! Managed Node.js discovery/version checking and Extension Host unresponsive watchdog.

use crate::rpc::resolve_node_binary;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const MINIMUM_NODE_MAJOR: u32 = 18;
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_UNRESPONSIVE_THRESHOLD: Duration = Duration::from_secs(15);
pub const DEFAULT_MAX_AUTO_RESTARTS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for NodeVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

pub fn parse_node_version(raw: &str) -> Option<NodeVersion> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some(NodeVersion {
        major,
        minor,
        patch,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedNodeStatus {
    pub binary_path: Option<PathBuf>,
    pub version: Option<NodeVersion>,
    pub meets_minimum: bool,
    pub minimum_required_major: u32,
    pub discovery_source: String,
    pub error: Option<String>,
}

/// Inspect managed/system Node.js runtime location and enforce Node >= 18.
pub fn inspect_node_runtime(
    override_path: Option<&Path>,
    managed_dir: Option<&Path>,
) -> ManagedNodeStatus {
    let (candidate, source) = if let Some(ov) = override_path {
        match resolve_node_binary(Some(ov)) {
            Ok(p) => (p, "override".to_string()),
            Err(e) => {
                return ManagedNodeStatus {
                    binary_path: None,
                    version: None,
                    meets_minimum: false,
                    minimum_required_major: MINIMUM_NODE_MAJOR,
                    discovery_source: "override".to_string(),
                    error: Some(e),
                };
            }
        }
    } else if let Some(mdir) = managed_dir {
        #[cfg(windows)]
        let rels = ["node/node.exe", "node/bin/node.exe", "node.exe"];
        #[cfg(not(windows))]
        let rels = ["node/bin/node", "node/node", "node"];
        let managed = rels.iter().map(|r| mdir.join(r)).find(|p| p.is_file());
        if let Some(p) = managed {
            (p, "managed".to_string())
        } else {
            match resolve_node_binary(None) {
                Ok(p) => (p, "system".to_string()),
                Err(e) => {
                    return ManagedNodeStatus {
                        binary_path: None,
                        version: None,
                        meets_minimum: false,
                        minimum_required_major: MINIMUM_NODE_MAJOR,
                        discovery_source: "none".to_string(),
                        error: Some(e),
                    };
                }
            }
        }
    } else {
        match resolve_node_binary(None) {
            Ok(p) => (p, "system".to_string()),
            Err(e) => {
                return ManagedNodeStatus {
                    binary_path: None,
                    version: None,
                    meets_minimum: false,
                    minimum_required_major: MINIMUM_NODE_MAJOR,
                    discovery_source: "none".to_string(),
                    error: Some(e),
                };
            }
        }
    };

    let output = Command::new(&candidate).arg("--version").output();
    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(ver) = parse_node_version(&stdout) {
                let ok = ver.major >= MINIMUM_NODE_MAJOR;
                ManagedNodeStatus {
                    binary_path: Some(candidate),
                    version: Some(ver),
                    meets_minimum: ok,
                    minimum_required_major: MINIMUM_NODE_MAJOR,
                    discovery_source: source,
                    error: if ok {
                        None
                    } else {
                        Some(format!(
                            "Node.js {} is below minimum required v{}.0.0",
                            ver, MINIMUM_NODE_MAJOR
                        ))
                    },
                }
            } else {
                ManagedNodeStatus {
                    binary_path: Some(candidate),
                    version: None,
                    meets_minimum: false,
                    minimum_required_major: MINIMUM_NODE_MAJOR,
                    discovery_source: source,
                    error: Some(format!("Unparseable Node.js version string: '{}'", stdout.trim())),
                }
            }
        }
        Ok(out) => ManagedNodeStatus {
            binary_path: Some(candidate),
            version: None,
            meets_minimum: false,
            minimum_required_major: MINIMUM_NODE_MAJOR,
            discovery_source: source,
            error: Some(format!(
                "Node.js exited with status {:?} during --version check",
                out.status.code()
            )),
        },
        Err(err) => ManagedNodeStatus {
            binary_path: Some(candidate),
            version: None,
            meets_minimum: false,
            minimum_required_major: MINIMUM_NODE_MAJOR,
            discovery_source: source,
            error: Some(format!("Failed executing Node.js: {}", err)),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum WatchdogHealth {
    Healthy {
        last_latency_ms: Option<u64>,
        restarts: u32,
    },
    Unresponsive {
        elapsed_ms: u64,
        consecutive_misses: u32,
        can_auto_restart: bool,
        restarts: u32,
    },
    RestartLimitReached {
        elapsed_ms: u64,
        restarts: u32,
    },
}

/// Monitors Extension Host heartbeat responsiveness and tracks restart attempts.
#[derive(Debug, Clone)]
pub struct ExtHostWatchdog {
    heartbeat_interval: Duration,
    unresponsive_threshold: Duration,
    max_auto_restarts: u32,
    last_ack_at: Instant,
    pending_ping_since: Option<Instant>,
    consecutive_misses: u32,
    restarts: u32,
    last_latency_ms: Option<u64>,
}

impl Default for ExtHostWatchdog {
    fn default() -> Self {
        Self::new(
            DEFAULT_HEARTBEAT_INTERVAL,
            DEFAULT_UNRESPONSIVE_THRESHOLD,
            DEFAULT_MAX_AUTO_RESTARTS,
        )
    }
}

impl ExtHostWatchdog {
    pub fn new(
        heartbeat_interval: Duration,
        unresponsive_threshold: Duration,
        max_auto_restarts: u32,
    ) -> Self {
        Self {
            heartbeat_interval,
            unresponsive_threshold,
            max_auto_restarts,
            last_ack_at: Instant::now(),
            pending_ping_since: None,
            consecutive_misses: 0,
            restarts: 0,
            last_latency_ms: None,
        }
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub fn unresponsive_threshold(&self) -> Duration {
        self.unresponsive_threshold
    }

    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    pub fn last_latency_ms(&self) -> Option<u64> {
        self.last_latency_ms
    }

    pub fn should_send_heartbeat(&self, now: Instant) -> bool {
        self.pending_ping_since.is_none()
            && now.saturating_duration_since(self.last_ack_at) >= self.heartbeat_interval
    }

    pub fn mark_ping_sent(&mut self, now: Instant) {
        if self.pending_ping_since.is_none() {
            self.pending_ping_since = Some(now);
        }
    }

    pub fn record_ack(&mut self, now: Instant, latency_ms: u64) {
        self.last_ack_at = now;
        self.pending_ping_since = None;
        self.consecutive_misses = 0;
        self.last_latency_ms = Some(latency_ms);
    }

    pub fn record_miss(&mut self) {
        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
    }

    pub fn check_health(&self, now: Instant) -> WatchdogHealth {
        let elapsed = self
            .pending_ping_since
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_else(|| now.saturating_duration_since(self.last_ack_at));
        if elapsed >= self.unresponsive_threshold || self.consecutive_misses >= 3 {
            let elapsed_ms = elapsed.as_millis() as u64;
            if self.restarts < self.max_auto_restarts {
                WatchdogHealth::Unresponsive {
                    elapsed_ms,
                    consecutive_misses: self.consecutive_misses,
                    can_auto_restart: true,
                    restarts: self.restarts,
                }
            } else {
                WatchdogHealth::RestartLimitReached {
                    elapsed_ms,
                    restarts: self.restarts,
                }
            }
        } else {
            WatchdogHealth::Healthy {
                last_latency_ms: self.last_latency_ms,
                restarts: self.restarts,
            }
        }
    }

    pub fn record_restart(&mut self, now: Instant) {
        self.restarts = self.restarts.saturating_add(1);
        self.last_ack_at = now;
        self.pending_ping_since = None;
        self.consecutive_misses = 0;
    }

    pub fn reset_restart_budget(&mut self, now: Instant) {
        self.restarts = 0;
        self.last_ack_at = now;
        self.pending_ping_since = None;
        self.consecutive_misses = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_node_version_and_watchdog_state_machine() {
        assert_eq!(
            parse_node_version("v20.11.1\n"),
            Some(NodeVersion {
                major: 20,
                minor: 11,
                patch: 1
            })
        );
        assert_eq!(
            parse_node_version("v18.0.0-nightly"),
            Some(NodeVersion {
                major: 18,
                minor: 0,
                patch: 0
            })
        );
        assert!(parse_node_version("not-a-version").is_none());

        let t0 = Instant::now();
        let mut wd = ExtHostWatchdog::new(Duration::from_millis(50), Duration::from_millis(150), 1);
        assert!(!wd.should_send_heartbeat(t0));
        let t1 = t0 + Duration::from_millis(60);
        assert!(wd.should_send_heartbeat(t1));
        wd.mark_ping_sent(t1);
        assert!(matches!(
            wd.check_health(t1 + Duration::from_millis(40)),
            WatchdogHealth::Healthy { .. }
        ));
        assert!(matches!(
            wd.check_health(t1 + Duration::from_millis(200)),
            WatchdogHealth::Unresponsive {
                can_auto_restart: true,
                restarts: 0,
                ..
            }
        ));
        wd.record_restart(t1 + Duration::from_millis(200));
        assert_eq!(wd.restarts(), 1);
        wd.mark_ping_sent(t1 + Duration::from_millis(210));
        assert!(matches!(
            wd.check_health(t1 + Duration::from_millis(400)),
            WatchdogHealth::RestartLimitReached { restarts: 1, .. }
        ));
    }
}
