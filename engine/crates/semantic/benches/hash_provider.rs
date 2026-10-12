//! Hash provider throughput (SEM-002 target: 10k symbol texts < 200 ms single thread).
//! Run: `cargo bench -p semantic --bench hash_provider`.

use std::time::Instant;

use semantic::bench::{synthetic_text, SplitMix};
use semantic::embedding::hash::embed_text;

fn main() {
    let mut rng = SplitMix::new(42);
    let texts: Vec<String> = (0..10_000).map(|_| synthetic_text(&mut rng)).collect();
    let started = Instant::now();
    let mut checksum = 0f64;
    for t in &texts {
        checksum += f64::from(embed_text(t, 768)[0]);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    println!("hash provider: 10000 texts in {ms:.1} ms (checksum {checksum:.4}, target < 200 ms)");
}
