//! `profile_version` (PROF-006): a hash of every miner version plus the inference code version.
//! Changing any miner's semantics (and bumping its version) changes the profile version, which
//! invalidates cached profiles and enters the snapshot fingerprint (INIT-012).

use review_core::version::ProfileVersion;

use crate::conventions::ConventionRegistry;

/// Bumped whenever role, layer or module inference (PROF-002) or the profile shape changes.
pub const INFERENCE_VERSION: u32 = 1;

/// The version of a set of `(miner id, version)` pairs.
pub fn profile_version_of(miners: &[(String, u32)]) -> ProfileVersion {
    let mut sorted = miners.to_vec();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rg.profile_version.v1\0");
    hasher.update(&INFERENCE_VERSION.to_le_bytes());
    for (id, version) in &sorted {
        hasher.update(&(id.len() as u64).to_le_bytes());
        hasher.update(id.as_bytes());
        hasher.update(&version.to_le_bytes());
    }
    let bytes = hasher.finalize();
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes.as_bytes()[..4]);
    // Positive and within PostgreSQL `integer`.
    ProfileVersion(u32::from_le_bytes(word) & 0x7fff_ffff)
}

/// The version of a registry's miners.
pub fn profile_version(registry: &ConventionRegistry) -> ProfileVersion {
    profile_version_of(&registry.versions())
}
