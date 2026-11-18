//! Curated offline Open VSX catalog & deterministic pure-Rust search fallback.

use super::types::{ExtensionMetadata, SearchResponse};

/// Return a curated set of representative extensions from Open VSX.
pub fn curated_catalog() -> Vec<ExtensionMetadata> {
    vec![
        ExtensionMetadata {
            namespace: "rust-lang".to_string(),
            name: "rust-analyzer".to_string(),
            version: "0.3.2100".to_string(),
            display_name: Some("rust-analyzer".to_string()),
            description: Some("Rust language support for Visual Studio Code".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/rust-lang/rust-analyzer/0.3.2100/file/rust-lang.rust-analyzer-0.3.2100.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "ms-python".to_string(),
            name: "python".to_string(),
            version: "2024.16.1".to_string(),
            display_name: Some("Python".to_string()),
            description: Some("Python language support with IntelliSense, linting, debugging, and Jupyter".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/ms-python/python/2024.16.1/file/ms-python.python-2024.16.1.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "esbenp".to_string(),
            name: "prettier-vscode".to_string(),
            version: "11.0.0".to_string(),
            display_name: Some("Prettier - Code formatter".to_string()),
            description: Some("Code formatter using prettier".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/esbenp/prettier-vscode/11.0.0/file/esbenp.prettier-vscode-11.0.0.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "dbaeumer".to_string(),
            name: "vscode-eslint".to_string(),
            version: "3.0.10".to_string(),
            display_name: Some("ESLint".to_string()),
            description: Some("Integrates ESLint JavaScript into VS Code".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/dbaeumer/vscode-eslint/3.0.10/file/dbaeumer.vscode-eslint-3.0.10.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "PKief".to_string(),
            name: "material-icon-theme".to_string(),
            version: "5.14.1".to_string(),
            display_name: Some("Material Icon Theme".to_string()),
            description: Some("Material Design Icons for Visual Studio Code".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/PKief/material-icon-theme/5.14.1/file/PKief.material-icon-theme-5.14.1.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "enkia".to_string(),
            name: "tokyo-night".to_string(),
            version: "1.0.9".to_string(),
            display_name: Some("Tokyo Night".to_string()),
            description: Some("A clean Visual Studio Code theme celebrating the lights of Tokyo".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/enkia/tokyo-night/1.0.9/file/enkia.tokyo-night-1.0.9.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "eamodio".to_string(),
            name: "gitlens".to_string(),
            version: "15.6.2".to_string(),
            display_name: Some("GitLens — Git supercharged".to_string()),
            description: Some("Supercharge Git within VS Code — visualize code authorship via Git blame".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/eamodio/gitlens/15.6.2/file/eamodio.gitlens-15.6.2.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
        ExtensionMetadata {
            namespace: "dracula-theme".to_string(),
            name: "theme-dracula".to_string(),
            version: "2.25.1".to_string(),
            display_name: Some("Dracula Official".to_string()),
            description: Some("Official Dracula Theme. A dark theme for many editors".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/dracula-theme/theme-dracula/2.25.1/file/dracula-theme.theme-dracula-2.25.1.vsix".to_string(),
            ),
            files: None,
            all_versions: None,
        },
    ]
}

/// Filter the curated offline catalog by case-insensitive substring query.
pub fn search_curated_catalog(query: &str) -> SearchResponse {
    let trimmed = query.trim().to_ascii_lowercase();
    let all = curated_catalog();
    let extensions: Vec<ExtensionMetadata> = if trimmed.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|ext| {
                let id = format!("{}.{}", ext.namespace, ext.name).to_ascii_lowercase();
                let disp = ext
                    .display_name
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let desc = ext
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                id.contains(&trimmed)
                    || ext.namespace.to_ascii_lowercase().contains(&trimmed)
                    || ext.name.to_ascii_lowercase().contains(&trimmed)
                    || disp.contains(&trimmed)
                    || desc.contains(&trimmed)
            })
            .collect()
    };
    let total_size = extensions.len();
    SearchResponse {
        total_size,
        extensions,
    }
}
