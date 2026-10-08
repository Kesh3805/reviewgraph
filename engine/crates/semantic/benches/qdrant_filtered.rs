//! Filtered recall and latency benchmark for Qdrant retrieval (SEM-009).
//!
//! Needs a live Qdrant: `QDRANT_URL=http://127.0.0.1:6333 cargo bench -p semantic --bench
//! qdrant_filtered`. `SEM_BENCH_FULL=1` uses the 1M-point corpus; the default is the CI-sized one.
//! Writes `benchmarks/perf/semantic/reports/{date}.md` and `.json` when `SEM_BENCH_REPORT_DIR` is
//! set. Uses (and deletes) the dedicated collection `rg_bench_filtered`.

use semantic::bench::{Bench, CorpusSpec};
use semantic::QdrantConfig;

#[tokio::main]
async fn main() {
    let Some(cfg) = QdrantConfig::from_lookup(|k| std::env::var(k).ok()) else {
        eprintln!("QDRANT_URL is not set; skipping the semantic benchmark");
        return;
    };
    let spec = if std::env::var("SEM_BENCH_FULL").is_ok() {
        CorpusSpec::full()
    } else {
        CorpusSpec::small()
    };
    let bench = match Bench::new(cfg, "rg_bench_filtered") {
        Ok(b) => b,
        Err(e) => {
            eprintln!("benchmark setup failed: {e}");
            std::process::exit(2);
        }
    };
    match bench.run(&spec).await {
        Ok(report) => {
            let md = report.markdown();
            println!("{md}");
            if let Ok(dir) = std::env::var("SEM_BENCH_REPORT_DIR") {
                let date = chrono_free_date();
                let base = std::path::Path::new(&dir).join(&date);
                let json = serde_json::to_string_pretty(&report).unwrap_or_default();
                if let Err(e) = std::fs::write(base.with_extension("md"), md)
                    .and_then(|()| std::fs::write(base.with_extension("json"), json))
                {
                    eprintln!("could not write the report: {e}");
                }
            }
        }
        Err(e) => {
            eprintln!("benchmark failed: {e}");
            std::process::exit(1);
        }
    }
}

/// `YYYY-MM-DD` (UTC) without a date library.
fn chrono_free_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
