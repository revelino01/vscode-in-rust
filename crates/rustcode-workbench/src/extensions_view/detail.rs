//! Extension Marketplace Detail Webview HTML renderer and `postMessage` action parser.

use rustcode_ext_host::manifest::{ExtensionManifest, LoadedExtension};
use rustcode_ext_host::marketplace::ExtensionMetadata;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExtensionDetailMessage {
    Install {
        extension_id: String,
    },
    Uninstall {
        extension_id: String,
    },
    Enable {
        extension_id: String,
    },
    Disable {
        extension_id: String,
    },
    SwitchPreRelease {
        extension_id: String,
        pre_release: bool,
    },
    Update {
        extension_id: String,
    },
}

pub fn parse_extension_detail_message(val: &Value) -> Option<ExtensionDetailMessage> {
    let msg_type = val.get("type").and_then(Value::as_str)?.trim();
    let extension_id = val
        .get("extensionId")
        .or_else(|| val.get("id"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if extension_id.is_empty() {
        return None;
    }
    match msg_type {
        "install" => Some(ExtensionDetailMessage::Install { extension_id }),
        "uninstall" => Some(ExtensionDetailMessage::Uninstall { extension_id }),
        "enable" => Some(ExtensionDetailMessage::Enable { extension_id }),
        "disable" => Some(ExtensionDetailMessage::Disable { extension_id }),
        "switchPreRelease" | "togglePreRelease" => Some(ExtensionDetailMessage::SwitchPreRelease {
            extension_id,
            pre_release: val
                .get("preRelease")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }),
        "update" => Some(ExtensionDetailMessage::Update { extension_id }),
        _ => None,
    }
}

pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn markdown_to_safe_html(markdown: &str) -> String {
    let mut html = String::new();
    let mut in_code_block = false;
    let mut in_list = false;

    for raw_line in markdown.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim_start().starts_with("```") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            if in_code_block {
                html.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                html.push_str("<pre><code>");
                in_code_block = true;
            }
            continue;
        }
        if in_code_block {
            html.push_str(&escape_html(line));
            html.push('\n');
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h3>{}</h3>\n", escape_html(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h2>{}</h2>\n", escape_html(rest)));
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h1>{}</h1>\n", escape_html(rest)));
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>\n", escape_html(rest)));
        } else {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<p>{}</p>\n", escape_html(trimmed)));
        }
    }
    if in_list {
        html.push_str("</ul>\n");
    }
    if in_code_block {
        html.push_str("</code></pre>\n");
    }
    html
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionDetailContext<'a> {
    pub extension_id: &'a str,
    pub display_name: &'a str,
    pub publisher: &'a str,
    pub version: &'a str,
    pub description: &'a str,
    pub installed: bool,
    pub enabled: bool,
    pub pre_release: bool,
    pub available_update: Option<&'a str>,
    pub metadata: Option<&'a ExtensionMetadata>,
    pub manifest: Option<&'a ExtensionManifest>,
    pub readme_md: Option<&'a str>,
    pub changelog_md: Option<&'a str>,
}

impl<'a> ExtensionDetailContext<'a> {
    pub fn from_loaded(
        ext: &'a LoadedExtension,
        extension_id: &'a str,
        enabled: bool,
        pre_release: bool,
        available_update: Option<&'a str>,
    ) -> Self {
        Self {
            extension_id,
            display_name: ext
                .manifest
                .display_name
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&ext.manifest.name),
            publisher: &ext.manifest.publisher,
            version: &ext.manifest.version,
            description: ext.manifest.description.as_deref().unwrap_or(""),
            installed: true,
            enabled,
            pre_release,
            available_update,
            metadata: None,
            manifest: Some(&ext.manifest),
            readme_md: None,
            changelog_md: None,
        }
    }
}

/// Render a rich Extension Detail HTML document matching VS Code's Marketplace view.
pub fn render_extension_detail_html(ctx: &ExtensionDetailContext<'_>) -> String {
    let ext_id_esc = escape_html(ctx.extension_id);
    let title_esc = escape_html(ctx.display_name);
    let pub_esc = escape_html(ctx.publisher);
    let ver_esc = escape_html(ctx.version);
    let desc_esc = escape_html(ctx.description);

    let downloads_str = ctx
        .metadata
        .and_then(|m| m.download_count)
        .map(format_download_count)
        .unwrap_or_else(|| "—".to_string());
    let rating_str = ctx
        .metadata
        .and_then(|m| m.average_rating)
        .map(|r| format!("{:.1} ★", r))
        .unwrap_or_else(|| "Unrated".to_string());
    let license_str = ctx
        .metadata
        .and_then(|m| m.license.as_deref())
        .or_else(|| ctx.manifest.and_then(|m| m.license.as_deref()))
        .unwrap_or("See repository");

    let mut action_buttons = String::new();
    if ctx.installed {
        let toggle_cmd = if ctx.enabled { "disable" } else { "enable" };
        let toggle_lbl = if ctx.enabled { "Disable" } else { "Enable" };
        action_buttons.push_str(&format!(
            "<button class=\"btn\" onclick=\"sendAction('{}')\">{}</button>",
            toggle_cmd, toggle_lbl
        ));
        action_buttons.push_str(
            "<button class=\"btn btn-danger\" onclick=\"sendAction('uninstall')\">Uninstall</button>",
        );
        if let Some(upd) = ctx.available_update {
            action_buttons.push_str(&format!(
                "<button class=\"btn btn-primary\" onclick=\"sendAction('update')\">Update to v{}</button>",
                escape_html(upd)
            ));
        }
        let pre_lbl = if ctx.pre_release {
            "Switch to Release Version"
        } else {
            "Switch to Pre-Release Version"
        };
        action_buttons.push_str(&format!(
            "<button class=\"btn\" onclick=\"sendAction('switchPreRelease', {{ preRelease: {} }})\">{}</button>",
            !ctx.pre_release, pre_lbl
        ));
    } else {
        action_buttons.push_str(
            "<button class=\"btn btn-primary\" onclick=\"sendAction('install')\">Install</button>",
        );
    }

    let contributions_html = render_contributions_section(ctx.manifest);
    let readme_html = match ctx.readme_md {
        Some(md) if !md.trim().is_empty() => markdown_to_safe_html(md),
        _ => format!("<p>{}</p>", desc_esc),
    };
    let changelog_html = match ctx.changelog_md {
        Some(md) if !md.trim().is_empty() => markdown_to_safe_html(md),
        _ => "<p>No changelog provided.</p>".to_string(),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>{title_esc}</title>
<style>
body {{ background:#1e1e1e; color:#cccccc; font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif; margin:0; padding:24px 32px; line-height:1.5; }}
.header {{ display:flex; flex-direction:column; gap:8px; border-bottom:1px solid #333; padding-bottom:16px; margin-bottom:20px; }}
.title-row {{ display:flex; align-items:baseline; gap:12px; flex-wrap:wrap; }}
.title-row h1 {{ margin:0; font-size:24px; color:#ffffff; }}
.version-badge {{ background:#2d2d30; border:1px solid #454545; border-radius:4px; padding:2px 8px; font-size:12px; color:#9cdcfe; }}
.meta-row {{ font-size:13px; color:#9d9d9d; display:flex; gap:16px; flex-wrap:wrap; }}
.actions {{ display:flex; gap:8px; flex-wrap:wrap; margin-top:8px; }}
.btn {{ background:#3a3d41; color:#ffffff; border:1px solid #454545; border-radius:3px; padding:5px 12px; cursor:pointer; font-size:13px; }}
.btn-primary {{ background:#0e639c; border-color:#1177bb; }}
.btn-danger {{ background:#5a1d1d; border-color:#8a2c2c; }}
.section {{ margin-bottom:24px; }}
.section h2 {{ border-bottom:1px solid #2d2d30; padding-bottom:4px; color:#e5e5e5; font-size:16px; }}
pre {{ background:#252526; border:1px solid #333; padding:10px; overflow-x:auto; border-radius:4px; }}
code {{ font-family:"Cascadia Code",Consolas,monospace; font-size:12px; }}
table {{ border-collapse:collapse; width:100%; font-size:13px; }}
th,td {{ border:1px solid #333; padding:6px 10px; text-align:left; }}
th {{ background:#252526; color:#cccccc; }}
</style>
</head>
<body>
<div class="header" data-extension-id="{ext_id_esc}">
  <div class="title-row">
    <h1>{title_esc}</h1>
    <span class="version-badge">v{ver_esc}</span>
  </div>
  <div class="meta-row">
    <span>Publisher: <strong>{pub_esc}</strong></span>
    <span>ID: <code>{ext_id_esc}</code></span>
    <span>Downloads: {downloads_str}</span>
    <span>Rating: {rating_str}</span>
    <span>License: {license_str}</span>
  </div>
  <div>{desc_esc}</div>
  <div class="actions">{action_buttons}</div>
</div>
<div class="section" id="readme">
  <h2>Details</h2>
  {readme_html}
</div>
<div class="section" id="contributions">
  <h2>Feature Contributions</h2>
  {contributions_html}
</div>
<div class="section" id="changelog">
  <h2>Changelog</h2>
  {changelog_html}
</div>
<script>
const vscodeApi = typeof acquireVsCodeApi === 'function' ? acquireVsCodeApi() : null;
function sendAction(type, extra) {{
  if (vscodeApi) {{
    vscodeApi.postMessage(Object.assign({{ type: type, extensionId: "{ext_id_esc}" }}, extra || {{}}));
  }}
}}
</script>
</body>
</html>"#,
        title_esc = title_esc,
        ext_id_esc = ext_id_esc,
        ver_esc = ver_esc,
        pub_esc = pub_esc,
        downloads_str = escape_html(&downloads_str),
        rating_str = escape_html(&rating_str),
        license_str = escape_html(license_str),
        desc_esc = desc_esc,
        action_buttons = action_buttons,
        readme_html = readme_html,
        contributions_html = contributions_html,
        changelog_html = changelog_html,
    )
}

fn format_download_count(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn render_contributions_section(manifest: Option<&ExtensionManifest>) -> String {
    let Some(m) = manifest else {
        return "<p>Install or load extension manifest to inspect feature contributions.</p>"
            .to_string();
    };
    let Some(c) = &m.contributes else {
        return "<p>No declarative contributions.</p>".to_string();
    };
    let mut rows = String::new();
    if let Some(cmds) = &c.commands {
        for cmd in cmds {
            rows.push_str(&format!(
                "<tr><td>Command</td><td><code>{}</code></td><td>{}</td></tr>\n",
                escape_html(&cmd.command),
                escape_html(&cmd.title)
            ));
        }
    }
    if let Some(langs) = &c.languages {
        for lang in langs {
            rows.push_str(&format!(
                "<tr><td>Language</td><td><code>{}</code></td><td>{}</td></tr>\n",
                escape_html(&lang.id),
                escape_html(&lang.extensions.join(", "))
            ));
        }
    }
    if let Some(themes) = &c.themes {
        for th in themes {
            rows.push_str(&format!(
                "<tr><td>Color Theme</td><td>{}</td><td>{}</td></tr>\n",
                escape_html(&th.label),
                escape_html(&th.ui_theme)
            ));
        }
    }
    if let Some(icons) = &c.icon_themes {
        for it in icons {
            rows.push_str(&format!(
                "<tr><td>File Icon Theme</td><td><code>{}</code></td><td>{}</td></tr>\n",
                escape_html(&it.id),
                escape_html(&it.label)
            ));
        }
    }
    if rows.is_empty() {
        return "<p>No declarative contributions.</p>".to_string();
    }
    format!(
        "<table><thead><tr><th>Kind</th><th>Identifier</th><th>Detail</th></tr></thead><tbody>\n{}</tbody></table>",
        rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_detail_html_escapes_and_post_message_parser() {
        let html = render_extension_detail_html(&ExtensionDetailContext {
            extension_id: "esbenp.prettier-vscode",
            display_name: "Prettier <script>alert(1)</script>",
            publisher: "esbenp",
            version: "11.0.0",
            description: "Code formatter",
            installed: true,
            enabled: true,
            pre_release: false,
            available_update: Some("11.1.0"),
            metadata: None,
            manifest: None,
            readme_md: Some("# Prettier\n- Formats JS & TS\n```\nprettier --write .\n```"),
            changelog_md: Some("## 11.1.0\n- Faster AST"),
        });
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("Update to v11.1.0"));
        assert!(html.contains("acquireVsCodeApi"));

        let parsed = parse_extension_detail_message(&json!({
            "type": "switchPreRelease",
            "extensionId": "esbenp.prettier-vscode",
            "preRelease": true
        }))
        .unwrap();
        assert_eq!(
            parsed,
            ExtensionDetailMessage::SwitchPreRelease {
                extension_id: "esbenp.prettier-vscode".to_string(),
                pre_release: true,
            }
        );
    }
}
