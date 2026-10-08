//! The single source of truth for `resolved_by → confidence` (CG-003).
//!
//! Every confidence value in the system comes from here: the linker, the framework mapper and
//! the type-checker provider all call [`confidence_of`] (or [`derived`] for edges computed from
//! other edges), so a calibration change is a one-file change with a [`LINKER_VERSION`] bump
//! (ADR-007). Values are fixed constants on purpose — runtime-configurable overrides are not
//! supported, because a value that can change at runtime cannot be benchmarked.
//!
//! [`Confidence::from_permille`] is `pub(crate)`: only this module and [`crate::codec`] may
//! build a confidence from raw permille, so a search for `from_permille` outside those two
//! files and tests finds nothing.

use std::fmt;

use schemars::gen::SchemaGenerator;
use schemars::schema::{Metadata, NumberValidation, Schema, SchemaObject};
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::edge::ResolvedBy;

/// Bumped when a value in [`TABLE`] changes (minor) or when the combination rules change
/// (major). Recorded in `snapshots.analyzer_versions["linker"]` (IDX-002) and as the
/// `linker_version` span attribute on `graph.link`.
pub const LINKER_VERSION: &str = "1.0.0";

/// Edge confidence in permille, `0..=1000`.
///
/// Integer permille rather than `f32` so equality, ordering and serialization are exact
/// (`0.95` and `950` can never drift apart in a snapshot comparison).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Confidence(u16);

impl Confidence {
    /// `0‰` — no evidence at all.
    pub const MIN: Self = Self(0);
    /// `1000‰` — certain.
    pub const MAX: Self = Self(1000);

    /// Wraps raw permille, saturating at [`Self::MAX`].
    ///
    /// Crate-private on purpose: outside this module and [`crate::codec`] a confidence must
    /// come from [`confidence_of`], [`derived`] or a deserialized edge.
    pub(crate) const fn from_permille(permille: u16) -> Self {
        Self(if permille > 1000 { 1000 } else { permille })
    }

    pub const fn as_permille(self) -> u16 {
        self.0
    }

    pub fn as_f32(self) -> f32 {
        f32::from(self.0) / 1000.0
    }

    /// Saturating conversion: non-finite and non-positive inputs become [`Self::MIN`],
    /// anything at or above `1.0` becomes [`Self::MAX`].
    ///
    /// This is the forgiving constructor used by adapters that receive a float from an
    /// analyzer. Use [`Self::try_from_f32`] when a bad value must be reported instead.
    pub fn from_f32(value: f32) -> Self {
        if !value.is_finite() || value <= 0.0 {
            return Self::MIN;
        }
        if value >= 1.0 {
            return Self::MAX;
        }
        Self((value * 1000.0).round() as u16)
    }

    /// Strict conversion: rejects `NaN`, negatives and values above `1.0`.
    pub fn try_from_f32(value: f32) -> Result<Self, ConfidenceError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ConfidenceError { value });
        }
        Ok(Self::from_f32(value))
    }

    pub const fn is_min(self) -> bool {
        self.0 == 0
    }

    pub const fn is_max(self) -> bool {
        self.0 == 1000
    }
}

/// Why a float could not become a [`Confidence`].
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("confidence {value} is not a finite value in 0..=1")]
pub struct ConfidenceError {
    pub value: f32,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}‰", self.0)
    }
}

impl Serialize for Confidence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u16::deserialize(deserializer)?;
        if raw > 1000 {
            return Err(D::Error::custom(format!("confidence {raw} > 1000")));
        }
        Ok(Self(raw))
    }
}

impl JsonSchema for Confidence {
    fn schema_name() -> String {
        "Confidence".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut object = SchemaObject {
            instance_type: Some(schemars::schema::InstanceType::Number.into()),
            ..SchemaObject::default()
        };
        object.number = Some(Box::new(NumberValidation {
            minimum: Some(0.0),
            maximum: Some(1000.0),
            ..NumberValidation::default()
        }));
        object.metadata = Some(Box::new(schemars::schema::Metadata {
            description: Some("Edge confidence in permille, 0..=1000.".to_owned()),
            ..Metadata::default()
        }));
        Schema::Object(object)
    }
}

impl From<Confidence> for f32 {
    fn from(c: Confidence) -> f32 {
        c.as_f32()
    }
}

/// The confidence of an edge resolved by `r` (target-architecture §3.1).
///
/// Exhaustive on purpose: adding a [`ResolvedBy`] variant does not compile until a value is
/// chosen here.
pub const fn confidence_of(r: ResolvedBy) -> Confidence {
    match r {
        ResolvedBy::Structural => Confidence::from_permille(1000),
        ResolvedBy::TypeChecker => Confidence::from_permille(1000),
        ResolvedBy::Import => Confidence::from_permille(950),
        ResolvedBy::ThisMember => Confidence::from_permille(950),
        ResolvedBy::Framework => Confidence::from_permille(900),
        ResolvedBy::DiConstructor => Confidence::from_permille(850),
        ResolvedBy::TypeAnnotation => Confidence::from_permille(800),
        ResolvedBy::NameUnique => Confidence::from_permille(600),
        ResolvedBy::Heuristic => Confidence::from_permille(500),
        ResolvedBy::NameAmbiguous => Confidence::from_permille(300),
    }
}

/// Confidence of a derived edge (a `TESTS` edge computed from a resolved call, a `HANDLED_BY`
/// edge computed from a route fact): the weaker of the derivation rule and its input.
///
/// Idempotent, so re-deriving from an already-derived confidence changes nothing.
pub const fn derived(rule: ResolvedBy, input: Confidence) -> Confidence {
    let by_rule = confidence_of(rule);
    if by_rule.0 <= input.0 {
        by_rule
    } else {
        input
    }
}

/// The whole table, for documentation and tests.
pub const TABLE: [(ResolvedBy, Confidence); 10] = [
    (
        ResolvedBy::Structural,
        confidence_of(ResolvedBy::Structural),
    ),
    (
        ResolvedBy::TypeChecker,
        confidence_of(ResolvedBy::TypeChecker),
    ),
    (ResolvedBy::Import, confidence_of(ResolvedBy::Import)),
    (
        ResolvedBy::ThisMember,
        confidence_of(ResolvedBy::ThisMember),
    ),
    (ResolvedBy::Framework, confidence_of(ResolvedBy::Framework)),
    (
        ResolvedBy::DiConstructor,
        confidence_of(ResolvedBy::DiConstructor),
    ),
    (
        ResolvedBy::TypeAnnotation,
        confidence_of(ResolvedBy::TypeAnnotation),
    ),
    (
        ResolvedBy::NameUnique,
        confidence_of(ResolvedBy::NameUnique),
    ),
    (ResolvedBy::Heuristic, confidence_of(ResolvedBy::Heuristic)),
    (
        ResolvedBy::NameAmbiguous,
        confidence_of(ResolvedBy::NameAmbiguous),
    ),
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The exact values of target-architecture §3.1, as documented.
    #[test]
    fn table_matches_target_architecture_3_1() {
        assert_eq!(confidence_of(ResolvedBy::TypeChecker).as_f32(), 1.0);
        assert_eq!(confidence_of(ResolvedBy::Structural).as_f32(), 1.0);
        assert_eq!(confidence_of(ResolvedBy::Import).as_f32(), 0.95);
        assert_eq!(confidence_of(ResolvedBy::ThisMember).as_f32(), 0.95);
        assert_eq!(confidence_of(ResolvedBy::Framework).as_f32(), 0.9);
        assert_eq!(confidence_of(ResolvedBy::DiConstructor).as_f32(), 0.85);
        assert_eq!(confidence_of(ResolvedBy::TypeAnnotation).as_f32(), 0.8);
        assert_eq!(confidence_of(ResolvedBy::NameUnique).as_f32(), 0.6);
        assert_eq!(confidence_of(ResolvedBy::Heuristic).as_f32(), 0.5);
        assert_eq!(confidence_of(ResolvedBy::NameAmbiguous).as_f32(), 0.3);
        assert_eq!(TABLE.len(), 10);
        for (resolved, confidence) in TABLE {
            assert_eq!(
                confidence,
                confidence_of(resolved),
                "{resolved} disagrees with confidence_of"
            );
        }
    }

    #[test]
    fn ordering_is_monotonic() {
        let order = [
            ResolvedBy::Structural,
            ResolvedBy::TypeChecker,
            ResolvedBy::Import,
            ResolvedBy::ThisMember,
            ResolvedBy::Framework,
            ResolvedBy::DiConstructor,
            ResolvedBy::TypeAnnotation,
            ResolvedBy::NameUnique,
            ResolvedBy::Heuristic,
            ResolvedBy::NameAmbiguous,
        ];
        for pair in order.windows(2) {
            assert!(
                confidence_of(pair[0]) >= confidence_of(pair[1]),
                "{} must be >= {}",
                pair[0],
                pair[1]
            );
        }
        assert!(confidence_of(ResolvedBy::TypeChecker) >= confidence_of(ResolvedBy::Import));
        assert!(confidence_of(ResolvedBy::NameAmbiguous) < confidence_of(ResolvedBy::NameUnique));
    }

    #[test]
    fn derived_takes_minimum() {
        let certain = Confidence::MAX;
        assert_eq!(derived(ResolvedBy::NameUnique, certain).as_permille(), 600);
        assert_eq!(derived(ResolvedBy::Import, certain).as_permille(), 950);
        assert_eq!(
            derived(ResolvedBy::Structural, Confidence::MIN),
            Confidence::MIN
        );
        assert_eq!(
            derived(ResolvedBy::NameAmbiguous, Confidence::MAX).as_permille(),
            300
        );
        assert_eq!(
            derived(ResolvedBy::Import, confidence_of(ResolvedBy::Framework)),
            confidence_of(ResolvedBy::Framework),
            "the input can be weaker than the rule"
        );
    }

    #[test]
    fn derived_is_idempotent() {
        for (resolved, _) in TABLE {
            for input in [Confidence::MIN, Confidence::MAX, Confidence::from_f32(0.5)] {
                let once = derived(resolved, input);
                assert_eq!(derived(resolved, once), once, "{resolved}");
            }
        }
    }

    #[test]
    fn confidence_rounding_permille() {
        assert_eq!(Confidence::from_f32(0.95).as_permille(), 950);
        assert_eq!(Confidence::from_f32(0.3).as_permille(), 300);
        assert_eq!(Confidence::from_f32(0.0).as_permille(), 0);
        assert_eq!(Confidence::from_f32(1.0).as_permille(), 1000);
        assert_eq!(Confidence::from_f32(0.85).as_permille(), 850);
        assert_eq!(Confidence::from_f32(0.8).as_permille(), 800);
        assert_eq!(Confidence::from_f32(0.9).as_permille(), 900);
        assert_eq!(Confidence::from_f32(0.6).as_permille(), 600);
        assert_eq!(Confidence::from_f32(0.5).as_permille(), 500);
        assert_eq!(Confidence::from_f32(f32::NAN), Confidence::MIN);
        assert_eq!(Confidence::from_f32(-1.0), Confidence::MIN);
        assert_eq!(Confidence::from_f32(2.0), Confidence::MAX);
        assert_eq!(Confidence::MAX.as_permille(), 1000);
        assert_eq!(Confidence::MIN.as_permille(), 0);
        assert!(Confidence::MIN.is_min());
        assert!(Confidence::MAX.is_max());
        assert_eq!(Confidence::MAX.as_f32(), 1.0);
        assert_eq!(f32::from(Confidence::from_f32(0.42)), 0.42);
        assert_eq!(Confidence::try_from_f32(0.42).unwrap().as_permille(), 420);
        assert!(
            Confidence::try_from_f32(f32::NAN).is_err(),
            "NaN is rejected"
        );
        assert_eq!(
            Confidence::try_from_f32(1.5),
            Err(ConfidenceError { value: 1.5 })
        );
        assert_eq!(
            Confidence::try_from_f32(-0.1),
            Err(ConfidenceError { value: -0.1 })
        );
        assert_eq!(Confidence::try_from_f32(0.0).unwrap(), Confidence::MIN);
        assert_eq!(Confidence::try_from_f32(1.0).unwrap(), Confidence::MAX);
    }

    #[test]
    fn confidence_serde_roundtrip_and_range_check() {
        let json = serde_json::to_string(&Confidence::from_f32(0.95)).unwrap();
        assert_eq!(json, "950");
        assert_eq!(
            serde_json::from_str::<Confidence>(&json)
                .unwrap()
                .as_permille(),
            950
        );
        assert!(serde_json::from_str::<Confidence>("1001").is_err());
        assert!(serde_json::from_str::<Confidence>("-1").is_err());
        assert_eq!(format!("{}", Confidence::from_f32(0.95)), "950‰");
        let schema = serde_json::to_value(schemars::schema_for!(Confidence)).unwrap();
        assert_eq!(schema["type"], "number");
        assert_eq!(schema["maximum"], serde_json::json!(1000.0));
    }

    #[test]
    fn linker_version_is_recorded_shape() {
        assert_eq!(LINKER_VERSION, "1.0.0");
        let parts: Vec<&str> = LINKER_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())));
    }

    #[test]
    fn resolved_by_is_exhaustive_in_the_table() {
        let mut covered = [false; 10];
        for (resolved, _) in TABLE {
            covered[resolved as usize] = true;
        }
        assert!(covered.iter().all(|b| *b), "every ResolvedBy is in TABLE");
    }
}
