//! Profile versioning (PROF-006): the version hashes every miner version plus the inference code
//! version, so changing any of them invalidates cached profiles.

/// Bumped when the architecture/role inference changes.
pub const INFERENCE_VERSION: u32 = 1;

/// Deterministic, order-independent version over `(miner id, miner version)` pairs.
/// Always at least 1 (the `repository_profiles.profile_version > 0` check).
pub fn profile_version(miners: &[(&str, u32)], inference_version: u32) -> u32 {
    let mut sorted: Vec<(&str, u32)> = miners.to_vec();
    sorted.sort_unstable();
    let mut h = blake3::Hasher::new();
    h.update(b"profile-version/v1");
    h.update(&inference_version.to_le_bytes());
    for (id, v) in sorted {
        h.update(&(id.len() as u32).to_le_bytes());
        h.update(id.as_bytes());
        h.update(&v.to_le_bytes());
    }
    let b = h.finalize();
    let raw = u32::from_le_bytes([
        b.as_bytes()[0],
        b.as_bytes()[1],
        b.as_bytes()[2],
        b.as_bytes()[3],
    ]);
    // Keep within i32 range (stored as `integer`) and positive.
    (raw & 0x7fff_ffff).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miner_version_bump_changes_profile_version() {
        let a = profile_version(&[("a", 1), ("b", 1)], INFERENCE_VERSION);
        let b = profile_version(&[("a", 2), ("b", 1)], INFERENCE_VERSION);
        let c = profile_version(&[("a", 1), ("b", 1)], INFERENCE_VERSION + 1);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(a > 0 && a <= i32::MAX as u32);
    }

    #[test]
    fn order_independent() {
        assert_eq!(
            profile_version(&[("a", 1), ("b", 2)], 1),
            profile_version(&[("b", 2), ("a", 1)], 1)
        );
    }
}
