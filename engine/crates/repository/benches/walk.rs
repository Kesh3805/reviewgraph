#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use criterion::{criterion_group, criterion_main, Criterion};
use repository::walk::{walk, WalkOptions};

fn generate(root: &std::path::Path, files: usize) {
    for i in 0..files {
        let dir = root.join(format!("pkg{}/mod{}", i % 50, i % 200));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("file{i}.ts")), "export const x = 1;\n").unwrap();
    }
}

fn bench_walk(c: &mut Criterion) {
    let tmp = tempfile::tempdir().unwrap();
    generate(tmp.path(), 50_000);
    let opts = WalkOptions::default();
    c.bench_function("walk_50k_files", |b| {
        b.iter(|| walk(tmp.path(), &opts).unwrap());
    });
}

criterion_group!(benches, bench_walk);
criterion_main!(benches);
