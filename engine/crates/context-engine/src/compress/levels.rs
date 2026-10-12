//! Default compression level per item kind and the next cheaper level (CTX-007).

use crate::model::{CompressionLevel, ItemKind};

/// A changed symbol longer than this many lines defaults to changed hunks only.
pub const CHANGED_FULL_MAX_LINES: u32 = 80;
/// A distance-1 related symbol longer than this defaults to a trimmed body.
pub const RELATED_FULL_MAX_LINES: u32 = 40;
/// A test longer than this defaults to a trimmed body.
pub const TEST_FULL_MAX_LINES: u32 = 60;
/// Distance 2+ symbols at or above this score keep a trimmed body instead of the signature.
pub const FAR_BODY_MIN_SCORE: f32 = 0.6;

/// The level an item starts at; budgeting may step down with [`cheaper`].
pub fn default_level(
    kind: ItemKind,
    distance: Option<u8>,
    score: f32,
    lines: u32,
) -> CompressionLevel {
    match kind {
        ItemKind::ChangedSymbol => {
            if lines > CHANGED_FULL_MAX_LINES {
                CompressionLevel::ChangedHunks
            } else {
                CompressionLevel::Full
            }
        }
        ItemKind::RelatedSymbol => match distance {
            Some(0 | 1) => {
                if lines > RELATED_FULL_MAX_LINES {
                    CompressionLevel::BodyTrimmed
                } else {
                    CompressionLevel::Full
                }
            }
            _ => {
                if score >= FAR_BODY_MIN_SCORE {
                    CompressionLevel::BodyTrimmed
                } else {
                    CompressionLevel::SignatureOnly
                }
            }
        },
        ItemKind::ClassSignature => CompressionLevel::SignatureOnly,
        ItemKind::Test => {
            if lines > TEST_FULL_MAX_LINES {
                CompressionLevel::BodyTrimmed
            } else {
                CompressionLevel::Full
            }
        }
        ItemKind::ApiEndpoint
        | ItemKind::Config
        | ItemKind::RuleDoc
        | ItemKind::Convention
        | ItemKind::SchemaChange
        | ItemKind::DependencyChange => CompressionLevel::Full,
    }
}

/// The next cheaper level, or `None` for [`CompressionLevel::Reference`].
pub fn cheaper(level: CompressionLevel) -> Option<CompressionLevel> {
    match level {
        CompressionLevel::Full => Some(CompressionLevel::BodyTrimmed),
        CompressionLevel::ChangedHunks | CompressionLevel::BodyTrimmed => {
            Some(CompressionLevel::SignatureOnly)
        }
        CompressionLevel::SignatureOnly => Some(CompressionLevel::Reference),
        CompressionLevel::Reference => None,
    }
}
