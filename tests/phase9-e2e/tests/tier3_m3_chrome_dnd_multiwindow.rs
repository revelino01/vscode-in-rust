use phase9_e2e::{create_test_window, dispatch, exec_cmd, key_down, mouse_down, mouse_move, mouse_up, render_window_scene, TempWorkspace};
use rustcode_app::cli::{parse_cli_args, CliAction, CliOptions, StartupMode};
use rustcode_app::multi_window::{
    OpenRequest, SingleInstanceBind, SingleInstanceIpc, WindowManager, WindowRouteDecision,
};
use rustcode_app::runtime::{save_workbench_state, restore_workbench_state};
use rustcode_platform::keybindings::Modifiers;
use rustcode_platform::storage::{InMemoryStorageBackend, StorageBackend};
use rustcode_platform::render::{FontMetrics, KeyCode, MouseButton, Point, Rect};
use rustcode_workbench::layout::PanelPosition;
use rustcode_workbench::titlebar::TitleBarView;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn tier3_m3_secondary_sidebar_panel_positions_titlebar_context_menu_zoom_zen() {
    let (ctx, mut win) = create_test_window();
    let ws = TempWorkspace::new("phase9_t3_chrome");
    let rs_file = ws.write_file("src/app.rs", "fn alpha() {}\nstruct Beta { v: i32 }\n");
    win.open_workspace(ws.path());
    win.open_path(&rs_file, false);

    let bounds = Rect::from_xywh(0.0, 0.0, 1280.0, 800.0);
    let mods_ctrl = Modifiers {
        ctrl: true,
        shift: false,
        alt: false,
        meta: false,
    };

    // TitleBar + Secondary Sidebar + Panel Left/Right
    exec_cmd(&ctx, &mut win, "workbench.action.toggleMenuBar");
    exec_cmd(&ctx, &mut win, "workbench.action.toggleAuxiliaryBar");
    exec_cmd(&ctx, &mut win, "workbench.action.togglePanel");
    exec_cmd(&ctx, &mut win, "workbench.action.positionPanelRight");
    assert!(win.view.layout.title_bar_visible);
    assert!(win.view.layout.secondary_sidebar_visible);
    assert_eq!(win.view.layout.panel_position, PanelPosition::Right);

    let reg = win.view.compute_regions(bounds);
    assert_eq!(reg.title_bar.height(), TitleBarView::HEIGHT);
    assert!(reg.secondary_sidebar.min_x() >= reg.panel.max_x() - 0.1);
    assert!(reg.panel.min_x() >= reg.editor_area.max_x() - 0.1);

    let scene = render_window_scene(&win, 1280.0, 800.0);
    assert!(scene.contains_text("OUTLINE"));
    assert!(scene.contains_text("alpha"));
    assert!(scene.contains_text("Beta"));
    assert!(scene.contains_text("File"));

    // Extension contributes.menus + when clause filtering + right click
    let mut menus = HashMap::new();
    menus.insert(
        "editor/context".to_string(),
        vec![rustcode_ext_host::manifest::MenuContribution {
            command: "ext.phase9Custom".to_string(),
            when: Some("editorTextFocus".to_string()),
            group: Some("navigation@0".to_string()),
            alt: None,
        }],
    );
    let mut titles = HashMap::new();
    titles.insert("ext.phase9Custom".to_string(), "Phase9 Custom Action".to_string());
    win.view.context_menu_registry.register_contributions(&menus, &titles);

    let ed_pt = Point::new(reg.editor_area.min_x() + 60.0, reg.editor_area.min_y() + 80.0);
    dispatch(&mut win, &mouse_down(ed_pt, MouseButton::Right), bounds);
    assert!(win.view.context_menu.is_open);
    let menu_scene = render_window_scene(&win, 1280.0, 800.0);
    assert!(menu_scene.contains_text("Phase9 Custom Action"));
    dispatch(&mut win, &key_down(KeyCode::Escape, Modifiers::none()), bounds);
    assert!(!win.view.context_menu.is_open);

    // Zoom Ctrl+= / Ctrl+0
    let base = FontMetrics::default_monospace();
    dispatch(&mut win, &key_down(KeyCode::Char('='), mods_ctrl), bounds);
    assert!(win.view.effective_font_metrics(&base).font_size > base.font_size);
    dispatch(&mut win, &key_down(KeyCode::Char('0'), mods_ctrl), bounds);
    assert_eq!(win.view.effective_font_metrics(&base), base);

    // Zen Mode chord Ctrl+K Z snapshot & restore
    dispatch(&mut win, &key_down(KeyCode::Char('k'), mods_ctrl), bounds);
    dispatch(&mut win, &key_down(KeyCode::Char('z'), Modifiers::none()), bounds);
    assert!(win.view.layout.zen_mode);
    assert_eq!(win.view.compute_regions(bounds).editor_area, bounds);
    dispatch(&mut win, &key_down(KeyCode::Char('k'), mods_ctrl), bounds);
    dispatch(&mut win, &key_down(KeyCode::Char('z'), Modifiers::none()), bounds);
    assert!(!win.view.layout.zen_mode);
    assert!(win.view.layout.sidebar_visible);
    assert!(win.view.layout.secondary_sidebar_visible);
    assert!(win.view.layout.panel_visible);
    assert!(win.view.layout.title_bar_visible);
}

#[test]
fn tier3_m3_tab_dnd_explorer_dnd_and_persistence_roundtrip() {
    let ws = TempWorkspace::new("phase9_t3_dnd");
    let dir_target = ws.mkdir_all("pkg");
    let f1 = ws.write_file("one.rs", "pub fn one() {}\n");
    let f2 = ws.write_file("two.rs", "pub fn two() {}\n");

    let (ctx, mut win) = create_test_window();
    let storage: Arc<dyn StorageBackend> = Arc::new(InMemoryStorageBackend::new());
    win = win.with_storage_backend(Arc::clone(&storage));
    win.open_workspace(ws.path());
    win.open_path(&f1, false);
    win.open_path(&f2, false);

    // 1. Tab D&D reorder + split drop
    let bounds = Rect::from_xywh(0.0, 0.0, 1280.0, 800.0);
    let ed_bounds = win.view.compute_regions(bounds).editor_area;
    let font = FontMetrics::default_monospace();
    let rects = win.view.editor_area.tab_rects_for_group(0, ed_bounds, &font);
    let start = Point::new(rects[0].1.min_x() + 15.0, rects[0].1.min_y() + 15.0);
    let right_split = Point::new(ed_bounds.max_x() - 25.0, ed_bounds.min_y() + 200.0);
    dispatch(&mut win, &mouse_down(start, MouseButton::Left), bounds);
    dispatch(&mut win, &mouse_move(right_split), bounds);
    dispatch(&mut win, &mouse_up(right_split, MouseButton::Left), bounds);
    assert_eq!(win.view.editor_area.model.groups.len(), 2);

    // 2. Explorer D&D move entry into pkg/
    let moved = win.view.explorer.move_entry(&f1, &dir_target).unwrap();
    assert_eq!(moved, dir_target.join("one.rs"));
    assert!(moved.exists());

    // 3. Layout persistence roundtrip
    win.view.layout.mutate(|l| {
        l.toggle_secondary_sidebar();
        l.set_panel_position(PanelPosition::Left);
        l.panel_visible = true;
        l.panel_width = 315.0;
        l.toggle_title_bar();
        l.zoom_in();
    });
    save_workbench_state(&win).unwrap();

    let mut win2 = ctx.create_window().with_storage_backend(storage);
    win2.open_workspace(ws.path());
    assert!(restore_workbench_state(&mut win2, Some(ws.path())).unwrap());
    assert!(win2.view.layout.secondary_sidebar_visible);
    assert_eq!(win2.view.layout.panel_position, PanelPosition::Left);
    assert_eq!(win2.view.layout.panel_width, 315.0);
    assert!(win2.view.layout.title_bar_visible);
    assert_eq!(win2.view.layout.zoom_level, 1.0);
}

#[test]
fn tier3_m3_multi_window_cli_and_single_instance_ipc() {
    let ws = TempWorkspace::new("phase9_t3_ipc");
    let opts = match parse_cli_args(["--new-window", "--reuse-window", "main.rs"]).unwrap() {
        CliAction::Run(o) => o,
        other => panic!("unexpected cli action: {other:?}"),
    };
    assert!(opts.new_window);
    assert!(opts.reuse_window);
    assert_eq!(opts.startup_mode(true), StartupMode::Desktop);

    let zero = match parse_cli_args::<[&str; 0], &str>([]).unwrap() {
        CliAction::Run(o) => o,
        other => panic!("unexpected cli action: {other:?}"),
    };
    assert_eq!(zero.startup_mode(true), StartupMode::Headless);

    let mut mgr = WindowManager::new();
    let root_a = ws.mkdir_all("proj_a");
    let id1 = mgr.register_window(vec![root_a.clone()]);
    let routed = mgr.route_request(OpenRequest {
        paths: vec![root_a.join("src/lib.rs")],
        goto: Vec::new(),
        new_window: false,
        reuse_window: false,
    });
    assert!(matches!(
        routed,
        WindowRouteDecision::OpenInExisting { window_id, .. } if window_id == id1
    ));

    let ipc_dir = ws.mkdir_all("ipc");
    let initial = OpenRequest::from_cli(&CliOptions::default());
    let server = match SingleInstanceIpc::try_bind_or_forward(&ipc_dir, &initial, None).unwrap() {
        SingleInstanceBind::Leader(s) => s,
        SingleInstanceBind::Forwarded => panic!("expected leader"),
    };
    let forward_req = OpenRequest {
        paths: vec![PathBuf::from("forwarded.rs")],
        goto: Vec::new(),
        new_window: true,
        reuse_window: false,
    };
    assert!(matches!(
        SingleInstanceIpc::try_bind_or_forward(&ipc_dir, &forward_req, None).unwrap(),
        SingleInstanceBind::Forwarded
    ));
    let drained = server.drain_requests();
    assert_eq!(drained, vec![forward_req]);
    drop(server);
}
