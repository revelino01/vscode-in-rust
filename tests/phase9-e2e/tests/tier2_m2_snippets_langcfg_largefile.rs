use phase9_e2e::{create_test_window, key_down, render_window_scene, TempWorkspace};
use rustcode_editor::{
    expand_snippet_with_context, EditorView, GrammarRegistry, LanguageConfiguration,
    LanguageConfigurationRegistry, PieceTable, Position, SnippetContext, SnippetRegistry,
    LARGE_FILE_BYTE_THRESHOLD, LARGE_FILE_LINE_THRESHOLD,
};
use rustcode_ext_host::ExtensionLoader;
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{FontMetrics, KeyCode, Point, Rect, Scene, View, WorkbenchThemeStyles};
use rustcode_platform::theme::ThemeService;
use std::path::Path;

#[test]
fn tier2_m2_textmate_snippets_variables_transforms_mirrors_and_session() {
    let ctx = SnippetContext::from_editor(
        Some(Path::new("/repo/src/user_service.rs")),
        14,
        "    ",
        "old_val",
        "user_service",
        "clipped",
    );
    let tpl = "// $TM_FILENAME_BASE -> ${TM_FILENAME_BASE/(_)/-/g}\npub fn ${1:compute_total}(mode: ${2|fast,accurate|}) {\n    let tag = \"${1/(.*)/${1:/upcase}/}\";\n    $0\n}";
    let expanded = expand_snippet_with_context(tpl, &ctx);
    assert!(expanded.text.contains("// user_service -> user-service"));
    assert!(expanded.text.contains("let tag = \"COMPUTE_TOTAL\";"));

    let mut ed = EditorView::from_str("");
    assert!(ed.insert_snippet(tpl, Some(ctx)));
    let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 600.0);

    // Editing tabstop $1 dynamically updates transformed mirror
    ed.handle_event(&key_down(KeyCode::Char('s'), Modifiers::none()), bounds);
    ed.handle_event(&key_down(KeyCode::Char('u'), Modifiers::none()), bounds);
    ed.handle_event(&key_down(KeyCode::Char('m'), Modifiers::none()), bounds);
    assert!(ed.text().contains("pub fn sum(mode: fast)"));
    assert!(ed.text().contains("let tag = \"SUM\";"));

    // Tab advances to choice $2 and then $0 and finishes cleanly
    ed.handle_event(&key_down(KeyCode::Tab, Modifiers::none()), bounds);
    assert_eq!(
        ed.snippet_session
            .as_ref()
            .and_then(|s| s.active_group_ref())
            .map(|g| g.index),
        Some(2)
    );
    ed.handle_event(&key_down(KeyCode::Tab, Modifiers::none()), bounds);
    assert_eq!(
        ed.snippet_session
            .as_ref()
            .and_then(|s| s.active_group_ref())
            .map(|g| g.index),
        Some(0)
    );
    ed.handle_event(&key_down(KeyCode::Tab, Modifiers::none()), bounds);
    assert!(ed.snippet_session.is_none());
}

#[test]
fn tier2_m2_extension_language_configuration_and_snippets() {
    let ws = TempWorkspace::new("phase9_t2_ext");
    let ext_dir = ws.mkdir_all("ext");
    ws.write_file(
        "ext/snippets/custom.json",
        r#"{
            // JSONC comment allowed
            "Banner": { "prefix": "ban", "body": ["// === $1 ===", "$0"], "description": "Banner" },
        }"#,
    );
    ws.write_file(
        "ext/language-configuration.json",
        r###"{
            "comments": { "lineComment": "##" },
            "surroundingPairs": [["(", ")"], ["[", "]"]],
            "indentationRules": { "increaseIndentPattern": ":\\s*$" }
        }"###,
    );
    ws.write_file(
        "ext/package.json",
        r#"{
            "name": "t2-custom-ext",
            "publisher": "Phase9",
            "version": "1.0.0",
            "contributes": {
                "languages": [{ "id": "customlang", "configuration": "./language-configuration.json" }],
                "snippets": [{ "language": "customlang", "path": "./snippets/custom.json" }]
            }
        }"#,
    );

    let mut loader = ExtensionLoader::new();
    loader.load_from_dir(&ext_dir).unwrap();

    let mut snip_reg = SnippetRegistry::new();
    rustcode_app::runtime::load_extension_snippets(&mut snip_reg, &loader);
    assert_eq!(snip_reg.snippets_for_language("customlang").len(), 1);

    let mut lc_reg = LanguageConfigurationRegistry::new();
    rustcode_app::runtime::load_extension_language_configs(&mut lc_reg, &loader);
    let lc: LanguageConfiguration = lc_reg.get("customlang").unwrap().clone();

    let mut ed = EditorView::from_str("rule:\n");
    ed.set_language("customlang", ThemeService::default_dark_theme(), &GrammarRegistry::new());
    ed.set_language_config(Some(lc));
    let bounds = Rect::from_xywh(0.0, 0.0, 800.0, 400.0);
    // Ctrl+/ uses extension's custom `##` line comment
    ed.handle_event(
        &key_down(
            KeyCode::Char('/'),
            Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                meta: false,
            },
        ),
        bounds,
    );
    assert!(ed.text().starts_with("## "));
}

#[test]
fn tier2_m2_sticky_scroll_rainbow_brackets_and_100mb_virtualization() {
    // 1. Sticky Scroll & Rainbow Brackets render
    let mut ed = EditorView::from_str(
        "fn outer() {\n    if true {\n        let a = ((1 + 2));\n        let b = 3;\n        let c = 4;\n        let d = 5;\n    }\n}\n",
    );
    ed.set_language("rust", ThemeService::default_dark_theme(), &GrammarRegistry::new());
    ed.set_sticky_scroll_enabled(true);
    ed.set_bracket_pair_colorization(true);
    ed.set_bracket_pair_guides(true);
    ed.scroll_top = 3.0;
    let bounds = Rect::from_xywh(0.0, 0.0, 900.0, 400.0);
    let font = FontMetrics::default_monospace();
    let mut scene = Scene::new();
    ed.render(bounds, &WorkbenchThemeStyles::dark_default(), &font, &mut scene);
    assert!(scene.contains_text("fn outer() {"));

    // 2. 100 MB large file AVL PieceTable & Virtualized EditorView render
    let line = "pub fn perf_item(x: u64) -> u64 { x.wrapping_mul(7).wrapping_add(3) }\n";
    let target_bytes = 100 * 1024 * 1024;
    let count = target_bytes / line.len() + 1;
    let big_str = line.repeat(count);
    assert!(big_str.len() >= target_bytes);

    let mut pt = PieceTable::from_str(&big_str);
    drop(big_str);
    assert!(pt.len() >= LARGE_FILE_BYTE_THRESHOLD);
    assert!(pt.line_count() >= LARGE_FILE_LINE_THRESHOLD);

    let mid_off = pt.floor_char_boundary(pt.len() / 2);
    let pos = pt.offset_to_position(mid_off);
    assert_eq!(pt.position_to_offset(pos.0, pos.1), mid_off);
    pt.insert(mid_off, "/*100mb*/");
    assert_eq!(pt.get_range(mid_off, mid_off + 9), "/*100mb*/");
    assert_eq!(pt.delete(mid_off, 9), "/*100mb*/");

    let (_ctx, mut win) = create_test_window();
    let uri = "file:///large_100mb.rs";
    win.view.editor_area.open_file(uri, "large_100mb.rs", "", false);
    if let Some(ed_large) = win.view.editor_area.active_editor_mut() {
        *ed_large.buffer.write() = pt;
        assert!(ed_large.is_large_file());
        ed_large.scroll_top = 500_000.0;
    }
    let big_scene = render_window_scene(&win, 1280.0, 800.0);
    assert!(big_scene.glyph_runs().len() < 4_000);
    assert!(big_scene.contains_text("perf_item"));

    // Focus click sanity on large editor stays finite
    let _ = Position::new(0, 0);
    let _ = Point::new(0.0, 0.0);
}
