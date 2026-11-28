//! Semver extension auto-update checker and applier (`extensions.autoUpdate` / `extensions.autoCheckUpdates`).

use super::installer::ExtensionInstaller;
use super::types::MarketplaceError;
use rustcode_platform::configuration::ConfigurationService;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemverVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl SemverVersion {
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim().trim_start_matches(['v', 'V']);
        if trimmed.is_empty() {
            return None;
        }
        let no_build = trimmed.split('+').next()?;
        let (core, pre) = match no_build.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (no_build, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        Some(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl PartialOrd for SemverVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemverVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                // Stable release beats pre-release of same tuple.
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

pub fn is_newer_version(installed: &str, available: &str) -> bool {
    match (SemverVersion::parse(installed), SemverVersion::parse(available)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => installed.trim() != available.trim() && !available.trim().is_empty(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUpdateAvailable {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub installed_version: String,
    pub available_version: String,
    pub pre_release: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ExtensionAutoUpdater;

impl ExtensionAutoUpdater {
    pub fn should_auto_check(config: &ConfigurationService) -> bool {
        config
            .get::<bool>("extensions.autoCheckUpdates")
            .unwrap_or(true)
    }

    pub fn should_auto_update(config: &ConfigurationService) -> bool {
        match config.get_value("extensions.autoUpdate") {
            Some(serde_json::Value::Bool(b)) => b,
            Some(serde_json::Value::String(s)) => {
                let low = s.trim().to_ascii_lowercase();
                low == "true" || low == "all"
            }
            _ => true,
        }
    }

    pub async fn check_for_updates(
        installer: &ExtensionInstaller,
        config: Option<&ConfigurationService>,
    ) -> Result<Vec<ExtensionUpdateAvailable>, MarketplaceError> {
        if let Some(cfg) = config {
            if !Self::should_auto_check(cfg) {
                return Ok(Vec::new());
            }
        }

        let installed = installer.list_installed();
        let mut updates = Vec::new();
        for ext in installed {
            if ext.manifest.publisher.trim().is_empty() {
                continue;
            }
            let pre_release = installer
                .read_installed_meta(&ext.root_dir)
                .map(|m| m.pre_release)
                .unwrap_or(false);
            let meta = if pre_release {
                installer
                    .client()
                    .get_prerelease_metadata(&ext.manifest.publisher, &ext.manifest.name)
                    .await
            } else {
                installer
                    .client()
                    .get_extension_metadata(&ext.manifest.publisher, &ext.manifest.name)
                    .await
            };
            let Ok(remote) = meta else {
                continue;
            };
            if is_newer_version(&ext.manifest.version, &remote.version) {
                updates.push(ExtensionUpdateAvailable {
                    id: ExtensionInstaller::extension_id(
                        &ext.manifest.publisher,
                        &ext.manifest.name,
                    ),
                    namespace: ext.manifest.publisher.clone(),
                    name: ext.manifest.name.clone(),
                    installed_version: ext.manifest.version.clone(),
                    available_version: remote.version,
                    pre_release,
                });
            }
        }
        Ok(updates)
    }

    pub async fn apply_update(
        installer: &ExtensionInstaller,
        update: &ExtensionUpdateAvailable,
    ) -> Result<crate::manifest::LoadedExtension, MarketplaceError> {
        installer
            .install_from_marketplace_with_options(
                &update.namespace,
                &update.name,
                Some(&update.available_version),
                update.pre_release,
            )
            .await
    }

    pub async fn check_and_apply_auto_updates(
        installer: &ExtensionInstaller,
        config: &ConfigurationService,
    ) -> Result<Vec<ExtensionUpdateAvailable>, MarketplaceError> {
        let updates = Self::check_for_updates(installer, Some(config)).await?;
        if !Self::should_auto_update(config) {
            return Ok(Vec::new());
        }
        let mut applied = Vec::new();
        for upd in updates {
            if Self::apply_update(installer, &upd).await.is_ok() {
                applied.push(upd);
            }
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_comparison_precedence() {
        assert!(is_newer_version("1.2.3", "1.2.4"));
        assert!(is_newer_version("1.2.3", "1.3.0"));
        assert!(is_newer_version("1.2.3-pre.1", "1.2.3"));
        assert!(!is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.2.3", "1.2.3"));
    }
}
