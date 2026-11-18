//! PieceTable Editor Latency Performance Benchmark
//!
//! Measures PieceTable keystroke latency across core editing scenarios:
//! - Sequential single-character typing (5,000 keystrokes)
//! - Sequential single-character backspace deletion (1,000 keystrokes)
//! - Chunk paste (20 KB code block insertion)
//! - Random edits (insertions & deletions across 10,000 lines)
//! - Line query & offset resolution across 10,000 lines
//! - Formats statistics and verifies < 3 ms keystroke latency target

use rustcode_editor::PieceTable;
use std::time::{Duration, Instant};

struct BenchmarkStats {
    name: &'static str,
    samples: Vec<Duration>,
}

impl BenchmarkStats {
    fn new(name: &'static str, mut samples: Vec<Duration>) -> Self {
        samples.sort();
        Self { name, samples }
    }

    fn min(&self) -> Duration {
        *self.samples.first().unwrap_or(&Duration::ZERO)
    }

    fn max(&self) -> Duration {
        *self.samples.last().unwrap_or(&Duration::ZERO)
    }

    fn mean(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let total: Duration = self.samples.iter().sum();
        total / self.samples.len() as u32
    }

    fn percentile(&self, p: f64) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let idx = ((self.samples.len() as f64 - 1.0) * (p / 100.0)).round() as usize;
        self.samples[idx]
    }

    fn ops_per_sec(&self) -> f64 {
        let mean_secs = self.mean().as_secs_f64();
        if mean_secs > 0.0 {
            1.0 / mean_secs
        } else {
            0.0
        }
    }

    fn print(&self) {
        println!(
            "{:<42} | Min: {:>8.2?} | Mean: {:>8.2?} | P50: {:>8.2?} | P95: {:>8.2?} | P99: {:>8.2?} | Max: {:>8.2?} | {:>10.0} ops/s",
            self.name,
            self.min(),
            self.mean(),
            self.percentile(50.0),
            self.percentile(95.0),
            self.percentile(99.0),
            self.max(),
            self.ops_per_sec(),
        );
    }
}

// Simple deterministic Pseudo-Random Number Generator (Xorshift64)
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
    println!(" RustCode Benchmark: PieceTable Editor Keystroke Latency (Target: < 3 ms / keystroke)");
    println!("=============================================================================================================");

    // 1. Sequential Single-Character Typing (5,000 keystrokes)
    let mut table = PieceTable::new();
    const TYPING_KEYSTROKES: usize = 5_000;
    let mut typing_samples = Vec::with_capacity(TYPING_KEYSTROKES);

    for i in 0..TYPING_KEYSTROKES {
        let ch = if i % 60 == 59 { "\n" } else { "x" };
        let offset = table.len();
        let start = Instant::now();
        table.insert(offset, ch);
        typing_samples.push(start.elapsed());
    }
    assert_eq!(table.len(), TYPING_KEYSTROKES);

    let typing_stats = BenchmarkStats::new("1. Sequential Typing (5,000 keystrokes)", typing_samples);
    typing_stats.print();

    // 2. Sequential Single-Character Backspace Deletion (1,000 keystrokes)
    const BACKSPACE_KEYSTROKES: usize = 1_000;
    let mut backspace_samples = Vec::with_capacity(BACKSPACE_KEYSTROKES);

    for _ in 0..BACKSPACE_KEYSTROKES {
        let offset = table.len() - 1;
        let start = Instant::now();
        let _ = table.delete(offset, 1);
        backspace_samples.push(start.elapsed());
    }
    assert_eq!(table.len(), TYPING_KEYSTROKES - BACKSPACE_KEYSTROKES);

    let backspace_stats = BenchmarkStats::new("2. Backspace Deletion (1,000 keystrokes)", backspace_samples);
    backspace_stats.print();

    // 3. Chunk Paste (20 KB code block)
    const PASTE_SIZE: usize = 20 * 1024; // 20 KB
    let paste_chunk = "pub fn execute_operation() -> Result<Data, Error> {\n    let val = 42;\n    Ok(val)\n}\n"
        .repeat(PASTE_SIZE / 80);

    const PASTE_ITERATIONS: usize = 50;
    let mut paste_samples = Vec::with_capacity(PASTE_ITERATIONS);

    for _ in 0..PASTE_ITERATIONS {
        let mut t = PieceTable::from_str("fn main() {\n    // Paste destination\n}\n");
        let paste_offset = 15;
        let start = Instant::now();
        t.insert(paste_offset, &paste_chunk);
        paste_samples.push(start.elapsed());
        assert!(t.len() >= paste_chunk.len());
    }

    let paste_stats = BenchmarkStats::new("3. Chunk Paste (20 KB insertion)", paste_samples);
    paste_stats.print();

    // 4. Random Edits Across 10,000 Lines
    let line_template = "    let entry = catalog.find_by_identifier(key_id, &mut query_cache)?;\n";
    let initial_content = line_template.repeat(10_000);
    let mut large_table = PieceTable::from_str(&initial_content);
    assert_eq!(large_table.line_count(), 10_001);

    let mut rng = SimpleRng::new(0xDEADBEEF_CAFE1234);
    const RANDOM_EDITS: usize = 2_000;
    let mut random_edit_samples = Vec::with_capacity(RANDOM_EDITS);

    for i in 0..RANDOM_EDITS {
        let current_len = large_table.len();
        let offset = rng.gen_range(0, current_len.saturating_sub(10));
        let start = Instant::now();

        if i % 2 == 0 {
            // Insertion
            large_table.insert(offset, "/* edit */");
        } else {
            // Deletion
            let del_len = rng.gen_range(1, 15);
            let _ = large_table.delete(offset, del_len);
        }

        random_edit_samples.push(start.elapsed());
    }

    let random_edit_stats = BenchmarkStats::new("4. Random Edits (2,000 ops on 10K lines)", random_edit_samples);
    random_edit_stats.print();

    // 5. Line Index & Offset Translation Queries
    const QUERY_COUNT: usize = 5_000;
    let mut query_samples = Vec::with_capacity(QUERY_COUNT);
    let total_lines = large_table.line_count();

    for _ in 0..QUERY_COUNT {
        let line = rng.gen_range(0, total_lines);
        let start = Instant::now();
        let (l, col) = large_table.offset_to_position(line * 10);
        let _ = (l, col);
        query_samples.push(start.elapsed());
    }

    let query_stats = BenchmarkStats::new("5. Position/Offset Query (10K lines)", query_samples);
    query_stats.print();

    println!("-------------------------------------------------------------------------------------------------------------");
    let mean_keystroke = typing_stats.mean();
    let target = Duration::from_millis(3);
    println!(
        "Sequential keystroke typing mean: {:?} (Target: < {:?}) => {}",
        mean_keystroke,
        target,
        if mean_keystroke < target { "PASSED" } else { "FAILED" }
    );
    println!("=============================================================================================================");

    assert!(
        mean_keystroke < target,
        "Keystroke typing latency {:?} exceeded target of {:?}",
        mean_keystroke,
        target
    );
}
