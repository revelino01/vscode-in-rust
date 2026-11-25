use phase8_e2e::{create_bridge_with_root, repo_root, E2eWindow, TempWorkspace};
use rustcode_base::context_key::{ContextKeyExpr, ContextKeyService};
use rustcode_base::map_ext_to_lang_id;
use rustcode_editor::TextDocument;
use rustcode_ext_host::provider::ExtStatusBarItem;
use rustcode_platform::fs::FileSystemService;
use rustcode_platform::render::{FontMetrics, GlyphRun, Point, Quad, Rect, Rgba, Scene, Shadow, Size};
use rustcode_protocols::codec::{
    decode_framed_message, encode_framed_message, parse_content_length_header,
    read_framed_message, write_framed_message, CodecError, MAX_FRAME_BYTES, MAX_HEADER_BYTES,
};
use rustcode_workbench::workspace::Workspace;
use serde_json::json;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, BufReader, ReadBuf};

struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    chunk_size: usize,
}

impl AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.pos >= self.data.len() {
            return Poll::Ready(Ok(()));
        }
        let remaining = self.data.len() - self.pos;
        let take = remaining.min(self.chunk_size).min(buf.remaining());
        let end = self.pos + take;
        buf.put_slice(&self.data[self.pos..end]);
        self.pos = end;
        Poll::Ready(Ok(()))
    }
}

/// [C1-01..C1-07, ADV-01] Shared bounded O(N) Content-Length framing codec.
#[tokio::test]
async fn t1_c1_codec_consolidation_and_bounded_framing() {
    // C1-01: Old duplicate lsp/codec.rs must stay deleted.
    let root = repo_root();
    assert!(
        !root.join("crates/rustcode-protocols/src/lsp/codec.rs").exists(),
        "[C1-01] Duplicate lsp/codec.rs must remain deleted"
    );

    // C1-02: Roundtrip empty JSON, multi-byte UTF-8, and pipelined frames.
    let mut wire = encode_framed_message(b"{}");
    let utf8_payload = "{\"msg\":\"日本語🚀\"}".as_bytes();
    wire.extend_from_slice(&encode_framed_message(utf8_payload));
    wire.extend_from_slice(&encode_framed_message(b"{\"seq\":3}"));

    let mut reader = BufReader::new(&wire[..]);
    let mut buf = Vec::new();
    assert_eq!(read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap(), b"{}");
    assert_eq!(
        read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap(),
        utf8_payload
    );
    assert_eq!(
        read_framed_message(&mut reader, &mut buf).await.unwrap().unwrap(),
        b"{\"seq\":3}"
    );
    assert!(read_framed_message(&mut reader, &mut buf).await.unwrap().is_none());

    // Async write_framed_message roundtrip over duplex.
    let (mut client_w, mut server_r) = tokio::io::duplex(4096);
    write_framed_message(&mut client_w, b"{\"ping\":true}").await.unwrap();
    drop(client_w);
    let mut duplex_buf = Vec::new();
    assert_eq!(
        read_framed_message(&mut server_r, &mut duplex_buf).await.unwrap().unwrap(),
        b"{\"ping\":true}"
    );

    // C1-03 & ADV-01: Header > 64 KiB rejected cleanly without unbounded allocation.
    let oversized_hdr = vec![b'X'; MAX_HEADER_BYTES + 256];
    let mut hdr_reader = ChunkedReader {
        data: oversized_hdr,
        pos: 0,
        chunk_size: 4096,
    };
    let mut hdr_buf = Vec::new();
    let hdr_err = read_framed_message(&mut hdr_reader, &mut hdr_buf).await.unwrap_err();
    assert!(matches!(hdr_err, CodecError::HeaderTooLarge(_)));

    // C1-04 & ADV-01: Frame > 64 MiB (incl. u32::MAX) rejected before body allocation.
    for len in [MAX_FRAME_BYTES + 1, 4_294_967_295] {
        let raw_over = format!("Content-Length: {len}\r\n\r\n");
        let mut cur = raw_over.as_bytes();
        let mut fbuf = Vec::new();
        let err = read_framed_message(&mut cur, &mut fbuf).await.unwrap_err();
        assert!(matches!(err, CodecError::FrameTooLarge { .. }));
    }
    assert_eq!(
        parse_content_length_header(format!("Content-Length: {MAX_FRAME_BYTES}").as_bytes()).unwrap(),
        MAX_FRAME_BYTES
    );

    // C1-05: ~60 KiB header delivered 17 bytes per poll + split \r\n|\r\n and \n|\n boundaries.
    let payload = b"{\"ok\":1}";
    let pad = "p".repeat(60 * 1024);
    let mut large_hdr_wire =
        format!("X-Pad: {pad}\r\nContent-Length: {}\r\n\r\n", payload.len()).into_bytes();
    large_hdr_wire.extend_from_slice(payload);
    let mut slow_reader = ChunkedReader {
        data: large_hdr_wire,
        pos: 0,
        chunk_size: 17,
    };
    let mut slow_buf = Vec::new();
    assert_eq!(
        read_framed_message(&mut slow_reader, &mut slow_buf).await.unwrap().unwrap(),
        payload
    );

    let mut lf_reader = ChunkedReader {
        data: b"content-length: 5\nContent-Type: application/vscode-jsonrpc\n\nhello".to_vec(),
        pos: 0,
        chunk_size: 1,
    };
    let mut lf_buf = Vec::new();
    assert_eq!(
        read_framed_message(&mut lf_reader, &mut lf_buf).await.unwrap().unwrap(),
        b"hello"
    );

    // C1-06 & C1-07: Malformed headers, NUL bytes, leading CRLF, mid-frame EOF.
    let mut bad_missing = b"Content-Type: application/json\r\n\r\n{}".to_vec();
    assert!(matches!(
        decode_framed_message(&mut bad_missing),
        Err(CodecError::MissingContentLength)
    ));
    let mut bad_neg = b"Content-Length: -5\r\n\r\n{}".to_vec();
    assert!(matches!(
        decode_framed_message(&mut bad_neg),
        Err(CodecError::InvalidContentLength(_))
    ));
    let mut bad_alpha = b"Content-Length: abc\r\n\r\n{}".to_vec();
    assert!(matches!(
        decode_framed_message(&mut bad_alpha),
        Err(CodecError::InvalidContentLength(_))
    ));
    let mut bad_plus = b"Content-Length: +5\r\n\r\nhello".to_vec();
    assert!(matches!(
        decode_framed_message(&mut bad_plus),
        Err(CodecError::InvalidContentLength(_))
    ));
    let mut bad_dup = b"Content-Length: 2\r\nContent-Length: 999\r\n\r\n{}".to_vec();
    assert!(decode_framed_message(&mut bad_dup).is_err());
    assert!(matches!(
        parse_content_length_header(b"Content-Length: +5\r\n"),
        Err(CodecError::InvalidContentLength(_))
    ));
    assert!(parse_content_length_header(b"Content-Length: 2\r\nContent-Length: 999\r\n").is_err());
    let mut bad_utf8 = b"X-Bad: \xFF\xFE\r\nContent-Length: 2\r\n\r\n{}".to_vec();
    assert!(matches!(
        decode_framed_message(&mut bad_utf8),
        Err(CodecError::InvalidHeader(_))
    ));
    let mut truncated = b"\r\nContent-Length: 10\r\n\r\nhi".as_slice();
    let mut tbuf = Vec::new();
    let eof_err = read_framed_message(&mut truncated, &mut tbuf).await.unwrap_err();
    assert!(matches!(
        eof_err,
        CodecError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}

/// [C2-01..C2-03] Unified `map_ext_to_lang_id` parity across `rustcode-base`, `rustcode-editor`, and `rustcode-ext-host`.
#[tokio::test]
async fn t1_c2_unified_map_ext_to_lang_id_parity() {
    let ws = TempWorkspace::new();
    let (bridge, _rpc) = create_bridge_with_root(ws.path());

    let cases: &[(&str, &str)] = &[
        ("file.hh", "cpp"),
        ("file.hxx", "cpp"),
        ("file.hpp", "cpp"),
        ("file.cc", "cpp"),
        ("file.cpp", "cpp"),
        ("file.CPP", "cpp"),
        ("notes.txt", "plaintext"),
        ("notes.TXT", "plaintext"),
        ("notes.text", "plaintext"),
        ("app.log", "plaintext"),
        ("README", "plaintext"),
        ("unknown.xyz", "plaintext"),
        ("run.sh", "shellscript"),
        ("run.bash", "shellscript"),
        ("lib.rs", "rust"),
        ("lib.RS", "rust"),
        ("index.js", "javascript"),
        ("index.mjs", "javascript"),
        ("index.cjs", "javascript"),
        ("app.ts", "typescript"),
        ("app.mts", "typescript"),
        ("app.cts", "typescript"),
        ("script.py", "python"),
        ("main.go", "go"),
        ("drv.c", "c"),
        ("drv.h", "c"),
        ("page.html", "html"),
        ("page.htm", "html"),
        ("style.css", "css"),
        ("Cargo.toml", "toml"),
        ("ci.yaml", "yaml"),
        ("ci.yml", "yaml"),
        ("pkg.json", "json"),
        ("doc.md", "markdown"),
    ];

    for &(filename, expected_lang) in cases {
        let ext = Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        assert_eq!(
            map_ext_to_lang_id(ext),
            expected_lang,
            "base map_ext_to_lang_id mismatch for {filename}"
        );

        let disk_path = ws.write_file(filename, "content\n");
        let fs_svc = FileSystemService::new();
        let doc = TextDocument::open_sync(&fs_svc, &disk_path).expect("TextDocument::open_sync");
        assert_eq!(
            doc.language_id, expected_lang,
            "TextDocument language_id mismatch for {filename}"
        );

        let rpc_doc = bridge
            .handle_rpc_message("workspace.openTextDocument", &json!({ "uri": ws.file_uri(filename) }))
            .await
            .expect("openTextDocument RPC");
        assert_eq!(
            rpc_doc["languageId"].as_str(),
            Some(expected_lang),
            "ExtHost bridge languageId mismatch for {filename}"
        );
    }
}

/// [C4-01..C4-03, ADV-05] Geometry bounds, FontMetrics validation, and Scene finite push guards.
#[test]
fn t1_c4_geometry_font_metrics_and_scene_bounds() {
    let bounds = Rect::from_xywh(10.0, 20.0, 100.0, 50.0);
    assert!(bounds.is_finite_positive());
    assert!(!Rect::from_xywh(0.0, 0.0, 0.0, 10.0).is_finite_positive());

    for bad in [
        Rect::new(Point::new(f32::NAN, 0.0), Size::new(10.0, 10.0)),
        Rect::new(Point::new(0.0, f32::INFINITY), Size::new(10.0, 10.0)),
        Rect::new(Point::new(0.0, f32::NEG_INFINITY), Size::new(10.0, 10.0)),
        Rect::new(Point::new(1.0e16, 0.0), Size::new(10.0, 10.0)),
    ] {
        assert!(!bad.is_finite());
        let clamped = bad.clamp_to_bounds(bounds);
        assert!(clamped.is_finite());
        assert!(clamped.min_x() <= clamped.max_x());
        assert!(clamped.min_y() <= clamped.max_y());
    }

    let larger = Rect::from_xywh(0.0, 0.0, 500.0, 500.0);
    assert_eq!(larger.clamp_to_bounds(bounds), bounds);

    let font = FontMetrics::default_monospace();
    assert!(font.is_finite());
    assert!(!(FontMetrics { line_height: 0.0, ..font.clone() }).is_finite());
    assert!(!(FontMetrics { char_width: -1.0, ..font.clone() }).is_finite());
    assert!(!(FontMetrics { font_size: f32::NAN, ..font }).is_finite());

    let mut scene = Scene::new();
    scene.push_finite_quad(Quad::new(Rect::new(Point::new(f32::NAN, 0.0), Size::new(10.0, 10.0))));
    scene.push_finite_text(GlyphRun::new(
        "bad".to_string(),
        Point::zero(),
        bounds,
        Rgba::black(),
        -2.0,
    ));
    scene.push_finite_shadow(Shadow {
        bounds,
        offset: Point::new(f32::INFINITY, 0.0),
        blur: 4.0,
        spread: 0.0,
        color: Rgba::black(),
    });
    assert!(scene.is_empty(), "all invalid scene nodes must be dropped");

    scene.push_finite_quad(Quad::new(bounds).with_background(Rgba::white()));
    scene.push_finite_text(GlyphRun::new(
        "ok".to_string(),
        Point::new(12.0, 22.0),
        bounds,
        Rgba::black(),
        14.0,
    ));
    scene.push_finite_shadow(Shadow {
        bounds,
        offset: Point::new(0.0, 2.0),
        blur: 4.0,
        spread: 0.0,
        color: Rgba::black(),
    });
    assert_eq!(scene.nodes().len(), 3);
}

/// [C5-01..C5-05, C6-01..C6-02, M6-03] Cleanup contracts, multi-root FileExplorer, sync_* parity, and ContextKey `[^]/]` char class.
#[tokio::test]
async fn t1_c5_c6_cleanup_and_context_key_char_class_m6_03() {
    let root = repo_root();

    // C5-03: No-op `gpui` feature removed from Cargo manifests.
    for rel in [
        "crates/rustcode-platform/Cargo.toml",
        "crates/rustcode-workbench/Cargo.toml",
        "crates/rustcode-app/Cargo.toml",
    ] {
        let toml = std::fs::read_to_string(root.join(rel)).unwrap();
        assert!(
            !toml.contains("gpui ="),
            "[C5-03] Stale `gpui` Cargo feature found in {rel}"
        );
    }

    // C5-01: No dead/uncalled AVL rotations in piece_table.rs (Phase 9 M2 wired live AVL rebalancing).
    let pt_src =
        std::fs::read_to_string(root.join("crates/rustcode-editor/src/model/piece_table.rs"))
            .unwrap();
    if pt_src.contains("fn rotate_left") || pt_src.contains("fn rotate_right") {
        assert!(
            pt_src.contains("fn rebalance")
                && pt_src.contains("Self::rotate_left(")
                && pt_src.contains("Self::rotate_right("),
            "[C5-01] AVL rotations in piece_table.rs must be live and called by rebalance()"
        );
    }

    // C5-04: Multi-root FileExplorer uses `roots` cleanly.
    let ws = TempWorkspace::new();
    let dir_a = ws.mkdir_all("alpha");
    let dir_b = ws.mkdir_all("beta");
    ws.write_file("alpha/one.rs", "fn one() {}\n");
    ws.write_file("beta/two.rs", "fn two() {}\n");

    let mut win = E2eWindow::new();
    let mut multi_ws = Workspace::from_folder(dir_a.clone());
    multi_ws.add_folder(dir_b.clone(), None);
    win.window.view.workspace = multi_ws;
    win.window
        .view
        .explorer
        .model
        .sync_from_workspace(&win.window.view.workspace);
    assert_eq!(win.window.view.explorer.model.roots.len(), 2);
    win.window.view.explorer.model.toggle_expanded(&dir_a);
    win.window.view.explorer.model.toggle_expanded(&dir_b);
    assert!(win.window.view.explorer.model.find_node(&dir_a.join("one.rs")).is_some());
    assert!(win.window.view.explorer.model.find_node(&dir_b.join("two.rs")).is_some());

    // C6-01 & C6-02: sync_* projections + legacy handle_rpc_message parity.
    let bridge = win.window.language_bridge.clone().unwrap();
    bridge.status_bar_manager.set_item(ExtStatusBarItem {
        id: "ext.c6.status".to_string(),
        text: "C6 Status".to_string(),
        tooltip: None,
        command: None,
        alignment: 2,
        priority: 5,
        visible: true,
    });
    bridge
        .test_manager
        .handle_rpc_call(
            "tests.createController",
            &json!({ "id": "c6-ctrl", "label": "C6 Suite" }),
        )
        .unwrap();
    bridge
        .test_manager
        .handle_rpc_call(
            "tests.updateItems",
            &json!({
                "controllerId": "c6-ctrl",
                "items": [{ "id": "t1", "label": "test_fail", "uri": "file:///a.rs" }]
            }),
        )
        .unwrap();
    bridge
        .test_manager
        .handle_rpc_call(
            "tests.createTestRun",
            &json!({ "runId": "r1", "controllerId": "c6-ctrl" }),
        )
        .unwrap();
    bridge
        .test_manager
        .handle_rpc_call(
            "tests.updateTestState",
            &json!({ "runId": "r1", "testId": "t1", "state": "failed" }),
        )
        .unwrap();
    let rpc_res = bridge
        .handle_rpc_message(
            "window.showInformationMessage",
            &json!({ "message": "C6 Toast" }),
        )
        .await;
    assert!(rpc_res.is_some());

    win.window.sync_status_bar();
    win.window.sync_window_manager();
    win.window.sync_test_manager();
    assert!(win
        .window
        .view
        .status_bar
        .model
        .right_items()
        .iter()
        .any(|i| i.id == "ext.c6.status"));
    assert_eq!(win.window.view.testing_view.failed_count(), 1);
    let rendered = win.render();
    assert!(rendered.contains_text("C6 Status"));
    assert!(rendered.contains_text("C6 Toast"));

    // M6-03: ContextKey `[^]/]` and `[]a]` leading-bracket regex char classes.
    let ctx = ContextKeyService::new();
    ctx.set("resourceFilename", json!("hello_world.rs"));
    ContextKeyExpr::parse("resourceFilename =~ /^[^]/]+$/")
        .expect("[M6-03] /^[^]/]+$/ must parse without Unclosed regex literal");
    assert!(ctx.evaluate_expr("resourceFilename =~ /^[^]/]+$/"));

    ctx.set("resourceFilename", json!("dir/sub.rs"));
    assert!(!ctx.evaluate_expr("resourceFilename =~ /^[^]/]+$/"));

    ctx.set("name", json!("]"));
    ContextKeyExpr::parse("name =~ /[]a]/")
        .expect("[M6-03] /[]a]/ must parse cleanly");
    assert!(ctx.evaluate_expr("name =~ /[]a]/"));

    ctx.set("path", json!("abc/def"));
    ContextKeyExpr::parse(r"path =~ /^[a-z\/]+$/")
        .expect("[M6-03] escaped \\/ in char class must parse");
    assert!(ctx.evaluate_expr(r"path =~ /^[a-z\/]+$/"));
}
