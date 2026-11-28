//! Workspace extension recommendations parser (`.vscode/extensions.json`).

use rustcode_platform::configuration::strip_json_comments;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceExtensionsRecommendations {
    #[serde(default)]
    pub recommendations: Vec<String>,
    #[serde(default)]
    pub unwanted_recommendations: Vec<String>,
}

impl WorkspaceExtensionsRecommendations {
    pub fn parse_jsonc(raw: &str) -> Result<Self, String> {
        let stripped = strip_json_comments(raw);
        let parsed: Self = serde_json::from_str(&stripped)
            .map_err(|e| format!("Invalid .vscode/extensions.json: {}", e))?;
        Ok(Self {
            recommendations: dedup_preserving_order(parsed.recommendations),
            unwanted_recommendations: dedup_preserving_order(parsed.unwanted_recommendations),
        })
    }

    pub fn load_from_workspace(workspace_root: &Path) -> Result<Self, String> {
        let path = rustcode_platform::AppPaths::workspace_paths(workspace_root)
            .extensions_recommendations_file();
        match std::fs::read_to_string(&path) {
            Ok(content) => Self::parse_jsonc(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!(
                "Failed to read '{}': {}",
                path.display(),
                e
            )),
        }
    }

    /// Return recommended extension IDs not yet installed, not unwanted, and not dismissed.
    pub fn pending_recommendations(
        &self,
        installed_ids: &[String],
        dismissed_ids: &HashSet<String>,
    ) -> Vec<String> {
        let installed_lower: HashSet<String> = installed_ids
            .iter()
            .map(|id| id.trim().to_ascii_lowercase())
            .collect();
        let unwanted_lower: HashSet<String> = self
            .unwanted_recommendations
            .iter()
            .map(|id| id.trim().to_ascii_lowercase())
            .collect();
        let dismissed_lower: HashSet<String> = dismissed_ids
            .iter()
            .map(|id| id.trim().to_ascii_lowercase())
            .collect();

        self.recommendations
            .iter()
            .filter(|rec| {
                let low = rec.trim().to_ascii_lowercase();
                !low.is_empty()
                    && !installed_lower.contains(&low)
                    && !unwanted_lower.contains(&low)
                    && !dismissed_lower.contains(&low)
            })
            .cloned()
            .collect()
    }
}

fn dedup_preserving_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let trimmed = item.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let low = trimmed.to_ascii_lowercase();
        if seen.insert(low) {
            out.push(trimmed);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_extensions_recommendations_filtering() {
        let raw = r#"{
            // Recommended extensions for Rust + TS
            "recommendations": [
                "rust-lang.rust-analyzer",
                "esbenp.prettier-vscode",
                "dbaeumer.vscode-eslint",
            ],
            "unwantedRecommendations": [
                "esbenp.prettier-vscode",
            ]
        }"#;
        let recs = WorkspaceExtensionsRecommendations::parse_jsonc(raw).unwrap();
        assert_eq!(recs.recommendations.len(), 3);
        let installed = vec!["RUST-LANG.rust-analyzer".to_string()];
        let dismissed = HashSet::new();
        let pending = recs.pending_recommendations(&installed, &dismissed);
        assert_eq!(pending, vec!["dbaeumer.vscode-eslint".to_string()]);
    }
}
