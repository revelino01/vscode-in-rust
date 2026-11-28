#![allow(dead_code)]

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

/// Returns the repository root (`.../vscode-in-rust`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tests/phase10-ext-compat must sit two levels below repo root")
        .to_path_buf()
}

/// RAII temporary workspace/sandbox directory removed automatically on drop.
pub struct TempTestDir {
    root: PathBuf,
}

impl TempTestDir {
    pub fn new(prefix: &str) -> Self {
        let dir_name = format!("{prefix}_{}_{}", std::process::id(), Uuid::new_v4().simple());
        let root = std::env::temp_dir().join(dir_name);
        std::fs::create_dir_all(&root).expect("failed to create temp test directory");
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn mkdir_all(&self, rel: impl AsRef<Path>) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        std::fs::create_dir_all(&target).expect("failed creating dir");
        target
    }

    pub fn write_file(&self, rel: impl AsRef<Path>, content: &str) -> PathBuf {
        let target = self.root.join(rel.as_ref());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("failed creating parent dirs");
        }
        std::fs::write(&target, content).expect("failed writing temp file");
        target
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Build an in-memory `.vsix` archive (`extension/package.json` + additional assets).
pub fn build_vsix_bytes(package_json: &Value, extra_files: &[(&str, &str)]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        let pkg_bytes = serde_json::to_vec_pretty(package_json).expect("serialize package.json");
        zip.start_file("extension/package.json", options)
            .expect("start extension/package.json");
        zip.write_all(&pkg_bytes)
            .expect("write extension/package.json");

        for (rel_path, content) in extra_files {
            let trimmed = rel_path.trim_start_matches(['/', '\\']).replace('\\', "/");
            let entry_name = format!("extension/{}", trimmed);
            zip.start_file(&entry_name, options)
                .expect("start extra VSIX entry");
            zip.write_all(content.as_bytes())
                .expect("write extra VSIX entry");
        }
        zip.finish().expect("finish zip");
    }
    cursor.into_inner()
}

/// Fixture definition for one of the 14 Phase 10 target top extensions.
pub struct Top14Fixture {
    pub id: &'static str,
    pub namespace: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub has_main_js: bool,
    pub test_command: Option<&'static str>,
    pub package_json: Value,
    pub vsix_bytes: Vec<u8>,
}

fn common_script(ext_label: &str, cmd_id: Option<&str>) -> String {
    let cmd_block = if let Some(cmd) = cmd_id {
        format!(
            r#"
  context.subscriptions.push(
    vscode.commands.registerCommand('{cmd}', async () => {{
      const prev = await context.secrets.get('probeSecret');
      await context.secrets.store('probeSecret', '{ext_label}:ok');
      const now = await context.secrets.get('probeSecret');
      return {{ ext: '{ext_label}', cmd: '{cmd}', prev: prev === undefined ? null : prev, stored: now }};
    }})
  );"#
        )
    } else {
        String::new()
    };

    format!(
        r#"const vscode = require('vscode');
exports.activate = async function(context) {{
  const log = vscode.window.createOutputChannel('{ext_label}', {{ log: true }});
  log.info('{ext_label} activated');
  context.subscriptions.push(log);{cmd_block}
  return {{ activated: '{ext_label}' }};
}};
"#
    )
}

type Top14SpecRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    bool,
    Option<&'static str>,
    Value,
    Vec<(&'static str, String)>,
);

/// Build `.vsix` fixtures representing all 14 Phase 10 target extensions.
pub fn build_top14_vsix_fixtures() -> Vec<Top14Fixture> {
    let specs: [Top14SpecRow; 14] = [
        (
            "esbenp.prettier-vscode",
            "esbenp",
            "prettier-vscode",
            "11.0.0",
            true,
            Some("prettier.forceFormatDocument"),
            json!({
                "name": "prettier-vscode",
                "publisher": "esbenp",
                "version": "11.0.0",
                "displayName": "Prettier - Code formatter",
                "description": "Code formatter using prettier",
                "categories": ["Formatters"],
                "keywords": ["prettier", "formatter"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "commands": [{ "command": "prettier.forceFormatDocument", "title": "Format Document With...", "category": "Prettier" }],
                    "configuration": { "title": "Prettier", "properties": { "prettier.semi": { "type": "boolean", "default": true } } }
                }
            }),
            vec![("extension.js", common_script("prettier-vscode", Some("prettier.forceFormatDocument")))],
        ),
        (
            "dbaeumer.vscode-eslint",
            "dbaeumer",
            "vscode-eslint",
            "3.0.10",
            true,
            Some("eslint.restart"),
            json!({
                "name": "vscode-eslint",
                "publisher": "dbaeumer",
                "version": "3.0.10",
                "displayName": "ESLint",
                "categories": ["Linters"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "commands": [{ "command": "eslint.restart", "title": "Restart ESLint Server", "category": "ESLint" }],
                    "problemMatchers": [{
                        "name": "eslint-custom",
                        "owner": "eslint",
                        "severity": "warning",
                        "pattern": { "regexp": "^(.*):(\\d+):(\\d+):\\s+(.*)$", "file": 1, "line": 2, "column": 3, "message": 4 }
                    }]
                }
            }),
            vec![("extension.js", common_script("vscode-eslint", Some("eslint.restart")))],
        ),
        (
            "eamodio.gitlens",
            "eamodio",
            "gitlens",
            "15.6.2",
            true,
            Some("gitlens.showQuickRepoStatus"),
            json!({
                "name": "gitlens",
                "publisher": "eamodio",
                "version": "15.6.2",
                "displayName": "GitLens — Git supercharged",
                "categories": ["SCM Providers", "Other"],
                "engines": { "vscode": "^1.82.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "viewsContainers": { "activitybar": [{ "id": "gitlens", "title": "GitLens", "icon": "resources/gitlens.svg" }] },
                    "views": { "gitlens": [{ "id": "gitlens.views.home", "name": "Home", "type": "webview" }] },
                    "viewsWelcome": [{ "view": "gitlens.views.home", "contents": "Open a Git repository to inspect blame history." }],
                    "submenus": [{ "id": "gitlens.submenu.commit", "label": "GitLens Actions" }],
                    "menus": { "editor/context": [{ "submenu": "gitlens.submenu.commit", "group": "gitlens" }] },
                    "commands": [{ "command": "gitlens.showQuickRepoStatus", "title": "GitLens: Show Repository Status" }]
                }
            }),
            vec![(
                "extension.js",
                r#"const vscode = require('vscode');
exports.activate = async function(context) {
  const providerDisp = vscode.workspace.registerTextDocumentContentProvider('gitlens-rev', {
    provideTextDocumentContent: (uri) => 'commit rev for ' + uri.path,
  });
  const wvDisp = vscode.window.registerWebviewViewProvider('gitlens.views.home', {
    resolveWebviewView(wv) {
      wv.title = 'GitLens Home';
      wv.webview.html = '<h1>GitLens Ready</h1>';
    }
  });
  const cmdDisp = vscode.commands.registerCommand('gitlens.showQuickRepoStatus', () => ({ repo: 'clean' }));
  context.subscriptions.push(providerDisp, wvDisp, cmdDisp);
  return { activated: 'gitlens' };
};
"#.to_string(),
            )],
        ),
        (
            "rust-lang.rust-analyzer",
            "rust-lang",
            "rust-analyzer",
            "0.3.2100",
            true,
            Some("rust-analyzer.analyzerStatus"),
            json!({
                "name": "rust-analyzer",
                "publisher": "rust-lang",
                "version": "0.3.2100",
                "displayName": "rust-analyzer",
                "categories": ["Programming Languages"],
                "engines": { "vscode": "^1.82.0" },
                "license": "MIT OR Apache-2.0",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "languages": [{ "id": "rust", "extensions": [".rs"] }],
                    "commands": [{ "command": "rust-analyzer.analyzerStatus", "title": "rust-analyzer: Status" }],
                    "problemMatchers": [{ "name": "ra-cargo", "base": "$rustc", "owner": "rust-analyzer" }]
                }
            }),
            vec![("extension.js", common_script("rust-analyzer", Some("rust-analyzer.analyzerStatus")))],
        ),
        (
            "ms-python.python",
            "ms-python",
            "python",
            "2024.16.1",
            true,
            Some("python.interpreterInfo"),
            json!({
                "name": "python",
                "publisher": "ms-python",
                "version": "2024.16.1",
                "displayName": "Python",
                "categories": ["Programming Languages", "Linters"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "languages": [{ "id": "python", "extensions": [".py"] }],
                    "commands": [{ "command": "python.interpreterInfo", "title": "Python: Interpreter Info" }]
                }
            }),
            vec![("extension.js", common_script("python", Some("python.interpreterInfo")))],
        ),
        (
            "golang.go",
            "golang",
            "go",
            "0.44.0",
            true,
            Some("go.version"),
            json!({
                "name": "go",
                "publisher": "golang",
                "version": "0.44.0",
                "displayName": "Go",
                "categories": ["Programming Languages", "Debuggers"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "languages": [{ "id": "go", "extensions": [".go"] }],
                    "commands": [{ "command": "go.version", "title": "Go: Show Version" }]
                }
            }),
            vec![("extension.js", common_script("go", Some("go.version")))],
        ),
        (
            "ms-vscode.cpptools",
            "ms-vscode",
            "cpptools",
            "1.22.11",
            true,
            Some("C_Cpp.ConfigurationEdit"),
            json!({
                "name": "cpptools",
                "publisher": "ms-vscode",
                "version": "1.22.11",
                "displayName": "C/C++",
                "categories": ["Programming Languages", "Debuggers"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "languages": [{ "id": "cpp", "extensions": [".cpp", ".hpp"] }],
                    "commands": [{ "command": "C_Cpp.ConfigurationEdit", "title": "C/C++: Edit Configurations" }],
                    "problemMatchers": [{ "name": "cpptools-gcc", "base": "$gcc", "owner": "cpp" }]
                }
            }),
            vec![("extension.js", common_script("cpptools", Some("C_Cpp.ConfigurationEdit")))],
        ),
        (
            "ms-azuretools.vscode-docker",
            "ms-azuretools",
            "vscode-docker",
            "1.29.3",
            true,
            Some("vscode-docker.registries.refresh"),
            json!({
                "name": "vscode-docker",
                "publisher": "ms-azuretools",
                "version": "1.29.3",
                "displayName": "Docker",
                "categories": ["Other"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "viewsContainers": { "activitybar": [{ "id": "docker", "title": "Docker", "icon": "resources/docker.svg" }] },
                    "views": { "docker": [{ "id": "vscode-docker.views.containers", "name": "Containers", "type": "tree" }] },
                    "jsonValidation": [{ "fileMatch": "docker-compose*.json", "url": "./schemas/compose-schema.json" }],
                    "commands": [{ "command": "vscode-docker.registries.refresh", "title": "Docker: Refresh Registries" }]
                }
            }),
            vec![
                ("extension.js", common_script("vscode-docker", Some("vscode-docker.registries.refresh"))),
                ("schemas/compose-schema.json", r#"{ "type": "object" }"#.to_string()),
            ],
        ),
        (
            "PKief.material-icon-theme",
            "PKief",
            "material-icon-theme",
            "5.14.1",
            false,
            None,
            json!({
                "name": "material-icon-theme",
                "publisher": "PKief",
                "version": "5.14.1",
                "displayName": "Material Icon Theme",
                "categories": ["Themes"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "activationEvents": ["*"],
                "contributes": {
                    "iconThemes": [{
                        "id": "material-icon-theme",
                        "label": "Material Icon Theme",
                        "path": "./icons/material-icons.json"
                    }],
                    "productIconThemes": [{
                        "id": "material-product-icons",
                        "label": "Material Product Icons",
                        "path": "./icons/material-product-icons.json"
                    }]
                }
            }),
            vec![
                (
                    "icons/material-icons.json",
                    json!({
                        "iconDefinitions": {
                            "_rust": { "iconPath": "./rust.svg", "fontColor": "#dea584" },
                            "_folder_src": { "iconPath": "./folder-src.svg" }
                        },
                        "fileExtensions": { "rs": "_rust" },
                        "folderNames": { "src": "_folder_src" }
                    })
                    .to_string(),
                ),
                (
                    "icons/material-product-icons.json",
                    json!({
                        "fonts": [{ "id": "material-codicon", "src": [{ "path": "./codicon.woff", "format": "woff" }] }],
                        "iconDefinitions": { "explorer-view-icon": { "fontCharacter": "\\eb00" } }
                    })
                    .to_string(),
                ),
            ],
        ),
        (
            "dracula-theme.theme-dracula",
            "dracula-theme",
            "theme-dracula",
            "2.25.1",
            false,
            None,
            json!({
                "name": "theme-dracula",
                "publisher": "dracula-theme",
                "version": "2.25.1",
                "displayName": "Dracula Official",
                "categories": ["Themes"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "activationEvents": ["*"],
                "contributes": {
                    "themes": [{
                        "label": "Dracula",
                        "uiTheme": "vs-dark",
                        "path": "./themes/dracula.json"
                    }]
                }
            }),
            vec![(
                "themes/dracula.json",
                json!({
                    "name": "Dracula",
                    "type": "dark",
                    "colors": { "editor.background": "#282a36", "editor.foreground": "#f8f8f2" },
                    "tokenColors": []
                })
                .to_string(),
            )],
        ),
        (
            "enkia.tokyo-night",
            "enkia",
            "tokyo-night",
            "1.0.9",
            false,
            None,
            json!({
                "name": "tokyo-night",
                "publisher": "enkia",
                "version": "1.0.9",
                "displayName": "Tokyo Night",
                "categories": ["Themes"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "activationEvents": ["*"],
                "contributes": {
                    "themes": [{
                        "label": "Tokyo Night",
                        "uiTheme": "vs-dark",
                        "path": "./themes/tokyo-night.json"
                    }]
                }
            }),
            vec![(
                "themes/tokyo-night.json",
                json!({
                    "name": "Tokyo Night",
                    "type": "dark",
                    "colors": { "editor.background": "#1a1b26", "editor.foreground": "#a9b1d6" },
                    "tokenColors": []
                })
                .to_string(),
            )],
        ),
        (
            "yzhang.markdown-all-in-one",
            "yzhang",
            "markdown-all-in-one",
            "3.6.2",
            true,
            Some("markdown.extension.toc.create"),
            json!({
                "name": "markdown-all-in-one",
                "publisher": "yzhang",
                "version": "3.6.2",
                "displayName": "Markdown All in One",
                "categories": ["Other"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "commands": [{ "command": "markdown.extension.toc.create", "title": "Create Table of Contents" }],
                    "keybindings": [{ "command": "markdown.extension.toc.create", "key": "ctrl+alt+t", "when": "editorTextFocus" }]
                }
            }),
            vec![("extension.js", common_script("markdown-all-in-one", Some("markdown.extension.toc.create")))],
        ),
        (
            "usernamehw.errorlens",
            "usernamehw",
            "errorlens",
            "3.20.0",
            true,
            Some("errorLens.toggle"),
            json!({
                "name": "errorlens",
                "publisher": "usernamehw",
                "version": "3.20.0",
                "displayName": "Error Lens",
                "categories": ["Linters", "Other"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "commands": [{ "command": "errorLens.toggle", "title": "Error Lens: Toggle" }],
                    "configuration": { "title": "Error Lens", "properties": { "errorLens.enabled": { "type": "boolean", "default": true } } }
                }
            }),
            vec![("extension.js", common_script("errorlens", Some("errorLens.toggle")))],
        ),
        (
            "rangav.vscode-thunder-client",
            "rangav",
            "vscode-thunder-client",
            "2.27.3",
            true,
            Some("thunder-client.new-request"),
            json!({
                "name": "vscode-thunder-client",
                "publisher": "rangav",
                "version": "2.27.3",
                "displayName": "Thunder Client",
                "categories": ["Testing", "Other"],
                "engines": { "vscode": "^1.80.0" },
                "license": "MIT",
                "main": "./extension.js",
                "activationEvents": ["*"],
                "contributes": {
                    "viewsContainers": { "activitybar": [{ "id": "thunder-client", "title": "Thunder Client", "icon": "images/tc.svg" }] },
                    "views": { "thunder-client": [{ "id": "thunder-client.sidebar", "name": "Collections", "type": "tree" }] },
                    "commands": [{ "command": "thunder-client.new-request", "title": "New Request" }]
                }
            }),
            vec![(
                "extension.js",
                r#"const vscode = require('vscode');
exports.activate = async function(context) {
  const mem = new Map();
  const fsDisp = vscode.workspace.registerFileSystemProvider('tcfs', {
    onDidChangeFile: new vscode.EventEmitter().event,
    stat: () => ({ type: vscode.FileType.File, ctime: 0, mtime: 1, size: 4 }),
    readDirectory: () => [],
    createDirectory() {},
    readFile: (u) => mem.get(u.path) || Buffer.from('{}'),
    writeFile: (u, b) => { mem.set(u.path, Buffer.from(b)); },
    delete: (u) => { mem.delete(u.path); },
    rename() {},
  });
  const cmdDisp = vscode.commands.registerCommand('thunder-client.new-request', async () => {
    await vscode.workspace.fs.writeFile(vscode.Uri.parse('tcfs:///req.json'), Buffer.from('{"ok":1}'));
    const raw = await vscode.workspace.fs.readFile(vscode.Uri.parse('tcfs:///req.json'));
    await context.secrets.store('apiKey', 'tc_secret_123');
    const sec = await context.secrets.get('apiKey');
    return { payload: Buffer.from(raw).toString('utf8'), secret: sec };
  });
  context.subscriptions.push(fsDisp, cmdDisp);
  return { activated: 'vscode-thunder-client' };
};
"#.to_string(),
            )],
        ),
    ];

    specs
        .into_iter()
        .map(
            |(id, namespace, name, version, has_main_js, test_command, package_json, extra)| {
                let extra_refs: Vec<(&str, &str)> =
                    extra.iter().map(|(p, c)| (*p, c.as_str())).collect();
                let vsix_bytes = build_vsix_bytes(&package_json, &extra_refs);
                Top14Fixture {
                    id,
                    namespace,
                    name,
                    version,
                    has_main_js,
                    test_command,
                    package_json,
                    vsix_bytes,
                }
            },
        )
        .collect()
}
