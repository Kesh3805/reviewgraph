//! Generation profiles (`s`, `m`, `l`). The same values live in `benchmarks/perf/profiles/*.yaml`
//! and a test keeps the two in sync.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSpec {
    pub name: String,
    pub modules: u32,
    pub features_per_module: u32,
    /// Scales entity/DTO property counts and service method counts (100 = baseline).
    pub density_pct: u32,
    /// Used only for the disk-space precheck.
    pub est_bytes_per_file: u64,
    pub target_files: u64,
    pub target_symbols: u64,
}

impl ProfileSpec {
    pub fn total_features(&self) -> u64 {
        u64::from(self.modules) * u64::from(self.features_per_module)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    S,
    M,
    L,
}

impl Profile {
    pub fn spec(self) -> ProfileSpec {
        let (name, modules, density_pct, est, files, symbols) = match self {
            Profile::S => ("s", 4, 262, 3600, 1_000, 15_000),
            Profile::M => ("m", 40, 147, 3000, 10_000, 100_000),
            Profile::L => ("l", 400, 147, 3000, 100_000, 1_000_000),
        };
        ProfileSpec {
            name: name.to_owned(),
            modules,
            features_per_module: 30,
            density_pct,
            est_bytes_per_file: est,
            target_files: files,
            target_symbols: symbols,
        }
    }
}

impl FromStr for Profile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "s" => Ok(Profile::S),
            "m" => Ok(Profile::M),
            "l" => Ok(Profile::L),
            other => Err(format!("unknown profile `{other}` (expected s, m or l)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_profiles_match_builtin() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../profiles");
        for p in [Profile::S, Profile::M, Profile::L] {
            let spec = p.spec();
            let text = std::fs::read_to_string(dir.join(format!("{}.yaml", spec.name))).unwrap();
            let parsed: ProfileSpec = serde_yaml::from_str(&text).unwrap();
            assert_eq!(parsed, spec);
        }
    }

    #[test]
    fn unknown_profile_is_rejected() {
        assert!("xl".parse::<Profile>().is_err());
    }
}
