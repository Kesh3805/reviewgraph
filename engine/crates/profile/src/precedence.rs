//! Source precedence of review guidance (target-architecture §3.10): explicit policy beats
//! documented architecture beats inferred conventions beats generic defaults. History-derived
//! signals (HIST-003, HIST-004) rank below all of them: history never outranks explicit policy.

/// The authority of a piece of guidance; later variants outrank earlier ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Authority {
    HistoryDerived,
    Generic,
    Inferred,
    DocumentedArchitecture,
    ExplicitPolicy,
}

impl Authority {
    pub const fn outranks(self, other: Self) -> bool {
        (self as u8) > (other as u8)
    }
}

/// The candidate with the highest authority; the first wins a tie.
pub fn resolve<T>(candidates: Vec<(Authority, T)>) -> Option<T> {
    let mut best: Option<(Authority, T)> = None;
    for (authority, value) in candidates {
        let replace = best.as_ref().is_none_or(|(a, _)| authority.outranks(*a));
        if replace {
            best = Some((authority, value));
        }
    }
    best.map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_never_outranks_explicit_policy() {
        for other in [
            Authority::Generic,
            Authority::Inferred,
            Authority::DocumentedArchitecture,
            Authority::ExplicitPolicy,
        ] {
            assert!(!Authority::HistoryDerived.outranks(other));
        }
        assert!(Authority::ExplicitPolicy.outranks(Authority::HistoryDerived));
        let winner = resolve(vec![
            (Authority::HistoryDerived, "history says suppress"),
            (Authority::ExplicitPolicy, "policy says report"),
            (Authority::Inferred, "convention"),
        ]);
        assert_eq!(winner, Some("policy says report"));
    }

    #[test]
    fn ties_keep_the_first_candidate() {
        let winner = resolve(vec![(Authority::Inferred, 1), (Authority::Inferred, 2)]);
        assert_eq!(winner, Some(1));
        assert_eq!(resolve::<u8>(Vec::new()), None);
    }
}
