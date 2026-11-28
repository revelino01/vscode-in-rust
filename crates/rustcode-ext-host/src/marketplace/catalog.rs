//! Curated offline Open VSX catalog & deterministic pure-Rust search fallback.

use super::types::{ExtensionMetadata, SearchResponse};

/// Return a curated set of representative top extensions from Open VSX.
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
            download_count: Some(8_500_000),
            average_rating: Some(4.9),
            categories: vec!["Programming Languages".to_string()],
            license: Some("MIT OR Apache-2.0".to_string()),
            ..Default::default()
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
            download_count: Some(140_000_000),
            average_rating: Some(4.6),
            categories: vec!["Programming Languages".to_string(), "Linters".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(50_000_000),
            average_rating: Some(4.5),
            categories: vec!["Formatters".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(38_000_000),
            average_rating: Some(4.7),
            categories: vec!["Linters".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(25_000_000),
            average_rating: Some(4.9),
            categories: vec!["Themes".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(2_200_000),
            average_rating: Some(4.8),
            categories: vec!["Themes".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(36_000_000),
            average_rating: Some(4.7),
            categories: vec!["SCM Providers".to_string(), "Other".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
            download_count: Some(7_800_000),
            average_rating: Some(4.8),
            categories: vec!["Themes".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "golang".to_string(),
            name: "go".to_string(),
            version: "0.44.0".to_string(),
            display_name: Some("Go".to_string()),
            description: Some("Rich Go language support for Visual Studio Code".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/golang/go/0.44.0/file/golang.go-0.44.0.vsix".to_string(),
            ),
            download_count: Some(15_000_000),
            average_rating: Some(4.7),
            categories: vec!["Programming Languages".to_string(), "Debuggers".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "ms-vscode".to_string(),
            name: "cpptools".to_string(),
            version: "1.22.11".to_string(),
            display_name: Some("C/C++".to_string()),
            description: Some("C/C++ IntelliSense, debugging, and code browsing".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/ms-vscode/cpptools/1.22.11/file/ms-vscode.cpptools-1.22.11.vsix".to_string(),
            ),
            download_count: Some(72_000_000),
            average_rating: Some(4.4),
            categories: vec!["Programming Languages".to_string(), "Debuggers".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "ms-azuretools".to_string(),
            name: "vscode-docker".to_string(),
            version: "1.29.3".to_string(),
            display_name: Some("Docker".to_string()),
            description: Some("Makes it easy to create, manage, and debug containerized applications".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/ms-azuretools/vscode-docker/1.29.3/file/ms-azuretools.vscode-docker-1.29.3.vsix".to_string(),
            ),
            download_count: Some(34_000_000),
            average_rating: Some(4.7),
            categories: vec!["Other".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "yzhang".to_string(),
            name: "markdown-all-in-one".to_string(),
            version: "3.6.2".to_string(),
            display_name: Some("Markdown All in One".to_string()),
            description: Some("All you need to write Markdown (keyboard shortcuts, TOC, auto preview)".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/yzhang/markdown-all-in-one/3.6.2/file/yzhang.markdown-all-in-one-3.6.2.vsix".to_string(),
            ),
            download_count: Some(9_200_000),
            average_rating: Some(4.8),
            categories: vec!["Other".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "usernamehw".to_string(),
            name: "errorlens".to_string(),
            version: "3.20.0".to_string(),
            display_name: Some("Error Lens".to_string()),
            description: Some("Improve highlighting of errors, warnings and other language diagnostics".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/usernamehw/errorlens/3.20.0/file/usernamehw.errorlens-3.20.0.vsix".to_string(),
            ),
            download_count: Some(7_600_000),
            average_rating: Some(4.9),
            categories: vec!["Linters".to_string(), "Other".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
        },
        ExtensionMetadata {
            namespace: "rangav".to_string(),
            name: "vscode-thunder-client".to_string(),
            version: "2.27.3".to_string(),
            display_name: Some("Thunder Client".to_string()),
            description: Some("Lightweight Rest API Client Extension for Visual Studio Code".to_string()),
            download_url: Some(
                "https://open-vsx.org/api/rangav/vscode-thunder-client/2.27.3/file/rangav.vscode-thunder-client-2.27.3.vsix".to_string(),
            ),
            download_count: Some(5_400_000),
            average_rating: Some(4.8),
            categories: vec!["Testing".to_string(), "Other".to_string()],
            license: Some("MIT".to_string()),
            ..Default::default()
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
