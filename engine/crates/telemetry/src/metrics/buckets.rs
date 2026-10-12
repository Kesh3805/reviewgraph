//! Histogram bucket sets.

/// Bucket boundaries (seconds) for every `*_duration_seconds` and `queue_wait_seconds`
/// histogram: 50 ms to 15 minutes.
pub const DURATION_SECONDS: [f64; 13] = [
    0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 900.0,
];
