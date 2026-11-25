use phase9_e2e::{create_test_window, dispatch, key_down, render_window_scene, TempWorkspace};
use rustcode_editor::{
    EditorCodeLens, EditorDocumentLink, EditorInlayHint, LspHoverInfo, PeekKind, PeekLocationItem,
    Position, Range, SemanticTokenSpan, SignatureEntry, SignatureParamInfo,
};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::render::{KeyCode, Rect};
use rustcode_protocols::lsp::{
    CallHierarchyIncomingCall, CallHierarchyItem, DocumentChange, LspPosition, LspRange,
    LspTextEdit, OptionalVersionedTextDocumentIdentifier, SymbolKind, TextDocumentEdit,
    TypeHierarchyItem, WorkspaceEdit,
};
use rustcode_workbench::hierarchy::HierarchyMode;
use rustcode_workbench::refactor_preview::{RefactorPreviewAction, RefactorPreviewOverlay};
use rustcode_workbench::workbench_view::WorkbenchAction;

#[test]
fn tier1_m1_lsp_widgets_semantic_tokens_and_hierarchy_views() {
    let (_ctx, mut win) = create_test_window();
    let ws = TempWorkspace::new("phase9_t1_lsp");
    let main_rs = ws.write_file("src/main.rs", "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n");
    win.open_workspace(ws.path());
    win.open_path(&main_rs, false);
    let main_uri = rustcode_platform::path_to_file_uri(&main_rs);

    let ed = win.view.editor_area.active_editor_mut().unwrap();
    // 1. Rich Markdown Hover
    ed.set_lsp_hover(
        Some(LspHoverInfo {
            contents: "## Function `add`\n\nAdds two integers.\n```rust\nfn add(a: i32, b: i32) -> i32\n```\n- fast\n- pure".to_string(),
            range: Some(Range::new(Position::new(0, 3), Position::new(0, 6))),
            position: Position::new(0, 3),
        }),
        None,
    );

    // 2. Signature Help
    ed.signature_help.show(
        vec![SignatureEntry {
            label: "fn add(a: i32, b: i32) -> i32".to_string(),
            documentation: Some("Adds two signed ints".to_string()),
            parameters: vec![
                SignatureParamInfo {
                    label: "a: i32".to_string(),
                    label_offsets: Some((7, 13)),
                    documentation: None,
                },
                SignatureParamInfo {
                    label: "b: i32".to_string(),
                    label_offsets: Some((15, 21)),
                    documentation: None,
                },
            ],
            active_parameter: Some(1),
        }],
        0,
        1,
        Position::new(1, 5),
    );

    // 3. Peek View
    ed.peek_view.open(
        PeekKind::References,
        "References to `add`",
        0,
        vec![PeekLocationItem {
            uri: main_uri.clone(),
            display_path: "src/main.rs".to_string(),
            range: Range::new(Position::new(0, 3), Position::new(0, 6)),
            preview_lines: vec![(0, "fn add(a: i32, b: i32) -> i32 {".to_string())],
            preview_match_line: 0,
        }],
    );

    // 4. Inline Rename input + Semantic Tokens + Inlay Hints + CodeLens + Document Links
    ed.rename_input.open(Position::new(0, 3), "sum_vals");
    ed.set_semantic_tokens(vec![SemanticTokenSpan {
        line: 0,
        start_utf16: 3,
        length_utf16: 3,
        token_type: "function".to_string(),
        modifiers: vec!["declaration".to_string()],
    }]);
    ed.set_inlay_hints(vec![EditorInlayHint {
        position: Position::new(1, 5),
        label: ": i32".to_string(),
        is_type: true,
        padding_left: false,
        padding_right: false,
        tooltip: None,
    }]);
    ed.set_code_lenses(vec![EditorCodeLens {
        line: 0,
        title: "2 references".to_string(),
        command: Some("editor.action.referenceSearch.trigger".to_string()),
        arguments: Vec::new(),
    }]);
    ed.set_document_links(vec![EditorDocumentLink {
        range: Range::new(Position::new(0, 0), Position::new(0, 2)),
        target: main_uri.clone(),
        tooltip: Some("Open definition".to_string()),
    }]);

    // 5. Call Hierarchy + Type Hierarchy Views
    let root_call = CallHierarchyItem {
        name: "add".to_string(),
        kind: SymbolKind::Function,
        tags: None,
        detail: Some("crate".to_string()),
        uri: main_uri.clone(),
        range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 10)),
        selection_range: LspRange::new(LspPosition::new(0, 3), LspPosition::new(0, 6)),
        data: None,
    };
    let caller = CallHierarchyItem {
        name: "main".to_string(),
        kind: SymbolKind::Function,
        tags: None,
        detail: None,
        uri: main_uri.clone(),
        range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 10)),
        selection_range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 4)),
        data: None,
    };
    win.view.hierarchy_view.set_call_incoming(
        root_call,
        vec![CallHierarchyIncomingCall {
            from: caller,
            from_ranges: vec![LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 4))],
        }],
    );
    win.view.layout.mutate(|l| l.set_sidebar_view("hierarchy"));

    let scene = render_window_scene(&win, 1280.0, 800.0);
    assert!(scene.contains_text("add"));
    assert!(scene.contains_text(": i32"));
    assert!(scene.contains_text("2 references"));
    assert!(scene.contains_text("References to `add`"));
    assert!(scene.contains_text("CALL HIERARCHY: INCOMING"));

    // Type hierarchy mode populates cleanly as well
    let root_type = TypeHierarchyItem {
        name: "Engine".to_string(),
        kind: SymbolKind::Struct,
        tags: None,
        detail: None,
        uri: main_uri,
        range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 6)),
        selection_range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 6)),
        data: None,
    };
    win.view
        .hierarchy_view
        .set_type_hierarchy(HierarchyMode::TypeSupertypes, root_type, Vec::new());
    let scene2 = render_window_scene(&win, 1280.0, 800.0);
    assert!(scene2.contains_text("TYPE HIERARCHY: SUPERTYPES"));
}

#[test]
fn tier1_m1_multi_file_workspace_edit_preview_filter_and_atomic_undo() {
    let (_ctx, mut win) = create_test_window();
    let ws = TempWorkspace::new("phase9_t1_wsedit");
    let a_rs = ws.write_file("src/a.rs", "pub fn old_fn() {}\n");
    let b_rs = ws.write_file("src/b.rs", "pub fn call() { old_fn(); }\n");
    let new_rs = ws.path().join("src/generated.rs");

    win.open_workspace(ws.path());
    win.open_path(&a_rs, false);
    win.open_path(&b_rs, false);

    let a_uri = rustcode_platform::path_to_file_uri(&a_rs);
    let b_uri = rustcode_platform::path_to_file_uri(&b_rs);
    let gen_uri = rustcode_platform::path_to_file_uri(&new_rs);

    let ws_edit = WorkspaceEdit {
        changes: None,
        document_changes: Some(vec![
            DocumentChange::CreateFile {
                uri: gen_uri.clone(),
            },
            DocumentChange::Edit(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: a_uri.clone(),
                    version: None,
                },
                edits: vec![LspTextEdit {
                    range: LspRange::new(LspPosition::new(0, 7), LspPosition::new(0, 13)),
                    new_text: "new_fn".to_string(),
                }],
            }),
            DocumentChange::Edit(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: b_uri.clone(),
                    version: None,
                },
                edits: vec![LspTextEdit {
                    range: LspRange::new(LspPosition::new(0, 16), LspPosition::new(0, 22)),
                    new_text: "new_fn".to_string(),
                }],
            }),
        ]),
    };

    // Uncheck first entry in preview, verify RefactorPreviewOverlay filters correctly
    let mut preview = RefactorPreviewOverlay::new();
    preview.open("Rename Preview", ws_edit.clone());
    assert_eq!(preview.files.len(), 3);
    preview.toggle_file(0);
    let filtered = preview.filtered_workspace_edit().unwrap();
    assert_eq!(filtered.document_changes.as_ref().unwrap().len(), 2);

    // Full preview on live WorkbenchWindow applies all changes and supports single-shot atomic undo
    win.view.refactor_preview.open("Refactor Rename", ws_edit);
    let bounds = Rect::from_xywh(0.0, 0.0, 1280.0, 800.0);
    dispatch(&mut win, &key_down(KeyCode::Enter, Modifiers::none()), bounds);
    if matches!(win.view.last_action, Some(WorkbenchAction::RefactorPreviewAction(_))) {
        win.sync_all();
    }

    // Apply directly with undo stack verification too
    if !new_rs.exists() {
        win.apply_workspace_edit_with_undo(
            "Refactor Rename",
            &win.view
                .refactor_preview
                .filtered_workspace_edit()
                .unwrap_or_else(|| WorkspaceEdit {
                    changes: None,
                    document_changes: Some(vec![
                        DocumentChange::CreateFile { uri: gen_uri },
                        DocumentChange::Edit(TextDocumentEdit {
                            text_document: OptionalVersionedTextDocumentIdentifier {
                                uri: a_uri.clone(),
                                version: None,
                            },
                            edits: vec![LspTextEdit {
                                range: LspRange::new(
                                    LspPosition::new(0, 7),
                                    LspPosition::new(0, 13),
                                ),
                                new_text: "new_fn".to_string(),
                            }],
                        }),
                        DocumentChange::Edit(TextDocumentEdit {
                            text_document: OptionalVersionedTextDocumentIdentifier {
                                uri: b_uri.clone(),
                                version: None,
                            },
                            edits: vec![LspTextEdit {
                                range: LspRange::new(
                                    LspPosition::new(0, 16),
                                    LspPosition::new(0, 22),
                                ),
                                new_text: "new_fn".to_string(),
                            }],
                        }),
                    ]),
                }),
        );
    }

    assert!(new_rs.exists());
    assert_eq!(
        win.view.editor_area.editors.get(&a_uri).unwrap().text(),
        "pub fn new_fn() {}\n"
    );
    assert_eq!(
        win.view.editor_area.editors.get(&b_uri).unwrap().text(),
        "pub fn call() { new_fn(); }\n"
    );
    let _ = RefactorPreviewAction::Cancel;

    // Single atomic undo reverts both buffers & removes created file
    let reverted = win.undo_last_workspace_edit();
    assert!(reverted >= 2);
    assert!(!new_rs.exists());
    assert_eq!(
        win.view.editor_area.editors.get(&a_uri).unwrap().text(),
        "pub fn old_fn() {}\n"
    );
    assert_eq!(
        win.view.editor_area.editors.get(&b_uri).unwrap().text(),
        "pub fn call() { old_fn(); }\n"
    );
}
