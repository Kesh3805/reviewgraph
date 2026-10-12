//! Deterministic point ids (SEM-007, target-architecture §3.9).

use review_core::ids::{OrganizationId, RepositoryId};
use uuid::Uuid;

use crate::units::UnitKind;

/// Fixed namespace for point ids. Never change it: every stored point id derives from it.
pub const NAMESPACE_RG: Uuid = Uuid::from_u128(0x7267_7365_6d61_4e54_8000_0000_0000_0001);

/// `uuid_v5(NAMESPACE_RG, "{org}|{repo}|{kind}|{key}|{space_id}")`. The organization is part of
/// the id, so identical code in two tenants never shares a point.
pub fn point_id(
    org: OrganizationId,
    repo: RepositoryId,
    kind: UnitKind,
    key: &str,
    space_id: &str,
) -> Uuid {
    let name = format!("{org}|{repo}|{}|{key}|{space_id}", kind.as_str());
    Uuid::new_v5(&NAMESPACE_RG, name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_tenant_specific() {
        let (o1, o2, r) = (
            OrganizationId::new(),
            OrganizationId::new(),
            RepositoryId::new(),
        );
        let a = point_id(o1, r, UnitKind::Doc, "k", "hash-fh768-768");
        assert_eq!(a, point_id(o1, r, UnitKind::Doc, "k", "hash-fh768-768"));
        assert_ne!(a, point_id(o2, r, UnitKind::Doc, "k", "hash-fh768-768"));
        assert_ne!(a, point_id(o1, r, UnitKind::Doc, "k", "openai-x-1536"));
        assert_eq!(a.get_version_num(), 5);
    }
}
