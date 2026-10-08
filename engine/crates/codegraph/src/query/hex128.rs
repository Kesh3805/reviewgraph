//! `NodeKey` on the wire as 32 lowercase hex characters (CG-007/008/009).
//!
//! The engine's JSON Schema and its database column both want a short, sortable, URL-safe string
//! for a key, and `SymbolKey`'s `Display` already produces exactly that. One module owns the
//! conversion so the traversal, path and subgraph payloads cannot drift apart from each other or
//! from `graph-storage`'s hex form.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

use crate::node_id::NodeKey;

/// Serializes one key as its 32 lowercase hex characters.
pub fn serialize<S: Serializer>(key: &NodeKey, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&key.to_string())
}

/// Deserializes one key from its 32 lowercase hex characters.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NodeKey, D::Error> {
    let raw = String::deserialize(deserializer)?;
    raw.parse::<NodeKey>().map_err(D::Error::custom)
}

/// The same conversion for a `Vec` of keys.
pub mod vec {
    use super::*;
    use serde::ser::SerializeSeq;

    pub fn serialize<S: Serializer>(keys: &[NodeKey], serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(keys.len()))?;
        for key in keys {
            seq.serialize_element(&key.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<NodeKey>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|raw| raw.parse::<NodeKey>().map_err(D::Error::custom))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::node_id::NodeId;

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct Wrapper {
        #[serde(with = "super")]
        key: NodeKey,
        #[serde(with = "super::vec")]
        keys: Vec<NodeKey>,
    }

    #[test]
    fn keys_round_trip_as_lowercase_hex() {
        let key = NodeId::from_canonical("ts:src/a.ts#A/f/function").key();
        let wrapper = Wrapper {
            key,
            keys: vec![key],
        };
        let json = serde_json::to_string(&wrapper).unwrap();
        assert!(json.contains(&key.to_string()), "{json}");
        assert_eq!(key.to_string().len(), 32);
        assert_eq!(
            serde_json::from_str::<Wrapper>(&json).unwrap().keys,
            vec![key]
        );
    }

    #[test]
    fn a_malformed_key_is_rejected_on_the_way_in() {
        assert!(serde_json::from_str::<Wrapper>(r#"{"key":"not-hex","keys":[]}"#).is_err());
        assert!(serde_json::from_str::<Wrapper>(r#"{"key":"00","keys":["00"]}"#).is_err());
    }
}
