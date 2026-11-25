//! Large-File (100 MB) PieceTable & EditorView Virtualization Benchmark
//!
//! Benchmarks O(log n) AVL `PieceTable` operations and virtualized `EditorView`
//! rendering on a synthetic >= 100 MiB source buffer:
//! - Initial 100+ MiB buffer load (`PieceTable::from_str`)
//! - 2,000 random `offset_to_position` / `position_to_offset` queries
//! - 500 mid-file insert & delete keystrokes (target mean < 3 ms)
//! - 10 virtualized `EditorView::render` frames scrolled to line 500,000

use parking_lot::RwLock;
use rustcode_editor::{EditorView, PieceTable};
use rustcode_platform::bench_support::BenchmarkStats;
use rustcode_platform::render::{FontMetrics, Rect, Scene, View, WorkbenchThemeStyles};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn gen_range(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize % (max - min))
    }
}

fn main() {
    println!("=============================================================================================================");
    println!(" RustCode Benchmark: 100 MB Large-File PieceTable & Virtualized Viewport Render");
    println!("=============================================================================================================");

    let line = "pub fn compute_item(val: usize) -> usize { val.saturating_mul(3).saturating_add(1) }\n";
    let target_bytes: usize = 100 * 1024 * 1024;
    let repeat_count = target_bytes / line.len() + 1;
    let payload = line.repeat(repeat_count);
    assert!(payload.len() >= target_bytes);

    let load_start = Instant::now();
    let mut table = PieceTable::from_str(&payload);
    let load_elapsed = load_start.elapsed();
    let mib = payload.len() as f64 / (1024.0 * 1024.0);
    println!(
        "1. Load {:.2} MiB ({} lines) in {:?} ({:.1} MiB/s)",
        mib,
        table.line_count(),
        load_elapsed,
        mib / load_elapsed.as_secs_f64().max(1e-9)
    );
    drop(payload);

    // 2. 2,000 random position/offset queries across the entire 100+ MiB buffer
    let mut rng = SimpleRng::new(0x9E37_79B9_7F4A_7C15);
    const QUERIES: usize = 2_000;
    let mut query_samples = Vec::with_capacity(QUERIES);
    let total_len = table.len();
    for _ in 0..QUERIES {
        let raw_off = rng.gen_range(0, total_len.saturating_sub(1));
        let off = table.floor_char_boundary(raw_off);
        let start = Instant::now();
        let (l, c) = table.offset_to_position(off);
        let back = table.position_to_offset(l, c);
        query_samples.push(start.elapsed());
        assert_eq!(back, off);
    }
    let query_stats = BenchmarkStats::new("2. Random pos<->off (100+ MiB)", query_samples);
    query_stats.print_with_ops();

    // 3. 500 mid-file edits near 50 MiB offset
    const MID_EDITS: usize = 500;
    let mut edit_samples = Vec::with_capacity(MID_EDITS);
    let mid_base = table.floor_char_boundary(table.len() / 2);
    for i in 0..MID_EDITS {
        let start = Instant::now();
        if i % 2 == 0 {
            table.insert(mid_base, "x");
        } else {
            let _ = table.delete(mid_base, 1);
        }
        edit_samples.push(start.elapsed());
    }
    let edit_stats = BenchmarkStats::new("3. Mid-file 50 MiB edits (500 ops)", edit_samples);
    edit_stats.print_with_ops();

    // 4. Virtualized 1280x800 EditorView viewport render at line 500,000
    let buf = Arc::new(RwLock::new(table));
    let mut view = EditorView::new(buf).with_minimap(true);
    assert!(view.is_large_file());
    let max_line = view.buffer.read().line_count().saturating_sub(50);
    view.scroll_top = (500_000usize.min(max_line)) as f32;
    let bounds = Rect::from_xywh(0.0, 0.0, 1280.0, 800.0);
    let theme = WorkbenchThemeStyles::dark_default();
    let font = FontMetrics::default_monospace();

    let mut render_samples = Vec::with_capacity(10);
    for _ in 0..10 {
        let mut scene = Scene::new();
        let start = Instant::now();
        view.render(bounds, &theme, &font, &mut scene);
        render_samples.push(start.elapsed());
        assert!(!scene.glyph_runs().is_empty());
    }
    let render_stats = BenchmarkStats::new("4. Virtualized Viewport Render (10 frames)", render_samples);
    render_stats.print_with_ops();

    let mean_edit = edit_stats.mean();
    let target = Duration::from_millis(3);
    println!(
        "Mid-file edit mean: {:?} (Target: < {:?}) => {}",
        mean_edit,
        target,
        if mean_edit < target { "PASSED" } else { "FAILED" }
    );
    assert!(
        mean_edit < target,
        "100 MB mid-file edit latency {:?} exceeded < 3 ms target",
        mean_edit
    );
}
