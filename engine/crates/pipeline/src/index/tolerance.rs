//! Parse tolerance policy (IDX-003): how many failed files an index may absorb.

/// Outcome class of one file, as far as the policy cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcomeKind {
    Ok,
    Partial,
    Failed,
    Skipped,
}

/// Counters per outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeCounts {
    pub ok: u32,
    pub partial: u32,
    pub failed: u32,
    pub skipped: u32,
}

impl OutcomeCounts {
    pub fn record(&mut self, kind: FileOutcomeKind) {
        let slot = match kind {
            FileOutcomeKind::Ok => &mut self.ok,
            FileOutcomeKind::Partial => &mut self.partial,
            FileOutcomeKind::Failed => &mut self.failed,
            FileOutcomeKind::Skipped => &mut self.skipped,
        };
        *slot = slot.saturating_add(1);
    }

    /// Files the policy judges: everything attempted (skipped files are excluded).
    pub fn attempted(&self) -> u32 {
        self.ok
            .saturating_add(self.partial)
            .saturating_add(self.failed)
    }

    pub fn parsed(&self) -> u32 {
        self.ok.saturating_add(self.partial)
    }
}

/// The index failed because too many files could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("parse tolerance exceeded: {failed} of {total} files failed")]
pub struct ToleranceExceeded {
    pub failed: u32,
    pub total: u32,
}

/// Limits from `.review/config.yaml` `index.tolerance.*`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TolerancePolicy {
    pub max_failed_ratio: f32,
    pub max_failed_files: Option<u32>,
    pub min_parsed_files: u32,
}

impl Default for TolerancePolicy {
    fn default() -> Self {
        Self {
            max_failed_ratio: 0.05,
            max_failed_files: None,
            min_parsed_files: 1,
        }
    }
}

impl TolerancePolicy {
    /// An empty repository (nothing attempted) is acceptable; otherwise all three limits apply.
    pub fn check(&self, counts: &OutcomeCounts) -> Result<(), ToleranceExceeded> {
        let total = counts.attempted();
        if total == 0 {
            return Ok(());
        }
        let err = ToleranceExceeded {
            failed: counts.failed,
            total,
        };
        let ratio = counts.failed as f32 / total as f32;
        if ratio > self.max_failed_ratio {
            return Err(err);
        }
        if self.max_failed_files.is_some_and(|max| counts.failed > max) {
            return Err(err);
        }
        if counts.parsed() < self.min_parsed_files {
            return Err(err);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(ok: u32, partial: u32, failed: u32) -> OutcomeCounts {
        OutcomeCounts {
            ok,
            partial,
            failed,
            skipped: 7,
        }
    }

    #[test]
    fn within_limits_passes() {
        assert!(TolerancePolicy::default().check(&counts(96, 0, 4)).is_ok());
    }

    #[test]
    fn ratio_is_enforced() {
        let e = TolerancePolicy::default().check(&counts(90, 0, 10));
        assert_eq!(
            e,
            Err(ToleranceExceeded {
                failed: 10,
                total: 100
            })
        );
    }

    #[test]
    fn absolute_cap_is_enforced() {
        let p = TolerancePolicy {
            max_failed_files: Some(1),
            ..TolerancePolicy::default()
        };
        assert!(p.check(&counts(1000, 0, 2)).is_err());
        assert!(p.check(&counts(1000, 0, 1)).is_ok());
    }

    #[test]
    fn minimum_parsed_is_enforced() {
        let p = TolerancePolicy {
            max_failed_ratio: 1.0,
            ..TolerancePolicy::default()
        };
        assert!(p.check(&counts(0, 0, 3)).is_err());
        assert!(p.check(&counts(0, 1, 3)).is_ok());
    }

    #[test]
    fn empty_and_skipped_only_repositories_pass() {
        assert!(TolerancePolicy::default()
            .check(&OutcomeCounts::default())
            .is_ok());
        let only_skipped = OutcomeCounts {
            skipped: 5,
            ..OutcomeCounts::default()
        };
        assert!(TolerancePolicy::default().check(&only_skipped).is_ok());
    }

    #[test]
    fn record_counts_each_kind() {
        let mut c = OutcomeCounts::default();
        c.record(FileOutcomeKind::Ok);
        c.record(FileOutcomeKind::Partial);
        c.record(FileOutcomeKind::Failed);
        c.record(FileOutcomeKind::Skipped);
        assert_eq!((c.ok, c.partial, c.failed, c.skipped), (1, 1, 1, 1));
        assert_eq!(c.attempted(), 3);
    }
}
