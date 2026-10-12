//! Tenant scope and filter rendering (SEM-005).
//!
//! Every filter sent to Qdrant is produced by [`scoped`], which always starts `must` with
//! `organization_id == scope.org` and `repository_id ∈ scope.repos`. Callers can only add
//! conditions through [`ExtraFilter`], which has no `should` clause and rejects tenant keys.

use std::collections::BTreeSet;

use review_core::ids::{OrganizationId, RepositoryId};
use serde_json::Value;

use crate::error::Error;
use crate::filter::{fields, Cond, Filter};
use crate::qdrant::Payload;

/// A vector with at least one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    /// `None` when `v` is empty.
    pub fn new(v: Vec<T>) -> Option<Self> {
        if v.is_empty() {
            None
        } else {
            Some(Self(v))
        }
    }

    pub fn one(item: T) -> Self {
        Self(vec![item])
    }

    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false`; present for API symmetry with `len`.
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// The tenant a request acts for: one organization and the repositories it may touch.
/// Fields are private; the only constructors take both parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantScope {
    organization_id: OrganizationId,
    repository_ids: NonEmptyVec<RepositoryId>,
}

impl TenantScope {
    /// Repository ids are de-duplicated and sorted, so rendered filters are deterministic.
    pub fn new(org: OrganizationId, repos: NonEmptyVec<RepositoryId>) -> Self {
        let set: BTreeSet<RepositoryId> = repos.0.into_iter().collect();
        Self {
            organization_id: org,
            repository_ids: NonEmptyVec(set.into_iter().collect()),
        }
    }

    pub fn single(org: OrganizationId, repo: RepositoryId) -> Self {
        Self::new(org, NonEmptyVec::one(repo))
    }

    pub fn organization_id(&self) -> OrganizationId {
        self.organization_id
    }

    pub fn repository_ids(&self) -> &[RepositoryId] {
        self.repository_ids.as_slice()
    }

    pub fn contains(&self, repo: &RepositoryId) -> bool {
        self.repository_ids.0.contains(repo)
    }

    /// The same organization narrowed to one of its repositories.
    pub fn narrowed(&self, repo: RepositoryId) -> Option<Self> {
        self.contains(&repo)
            .then(|| Self::single(self.organization_id, repo))
    }
}

/// Additional conditions a caller may add to a scoped request. Cannot express `should` and
/// cannot mention tenant keys, so it can only narrow the tenant filter, never widen it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtraFilter {
    must: Vec<Cond>,
    must_not: Vec<Cond>,
}

fn reject_tenant_key(cond: &Cond) -> Result<(), Error> {
    match cond.key() {
        Some(k) if fields::TENANT_KEYS.contains(&k) => Err(Error::ScopeViolation(format!(
            "extra filters may not use the tenant key {k}"
        ))),
        _ => Ok(()),
    }
}

impl ExtraFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a required condition. Tenant keys are rejected.
    pub fn must(mut self, cond: Cond) -> Result<Self, Error> {
        reject_tenant_key(&cond)?;
        self.must.push(cond);
        Ok(self)
    }

    /// Adds an excluding condition. Tenant keys are rejected.
    pub fn must_not(mut self, cond: Cond) -> Result<Self, Error> {
        reject_tenant_key(&cond)?;
        self.must_not.push(cond);
        Ok(self)
    }

    pub fn is_empty(&self) -> bool {
        self.must.is_empty() && self.must_not.is_empty()
    }
}

/// Renders the only kind of filter that is ever sent: tenant conditions first, then `extra`.
pub(crate) fn scoped(scope: &TenantScope, extra: &ExtraFilter) -> Filter {
    let mut must = Vec::with_capacity(2 + extra.must.len());
    must.push(Cond::keyword(
        fields::ORGANIZATION_ID,
        scope.organization_id,
    ));
    must.push(Cond::any(fields::REPOSITORY_ID, scope.repository_ids()));
    must.extend(extra.must.iter().cloned());
    Filter {
        must,
        should: Vec::new(),
        must_not: extra.must_not.clone(),
    }
}

/// Whether a payload's tenant keys lie inside `scope`.
pub(crate) fn payload_in_scope(scope: &TenantScope, payload: &Payload) -> bool {
    let org_ok = payload
        .get(fields::ORGANIZATION_ID)
        .and_then(Value::as_str)
        .is_some_and(|s| s == scope.organization_id.to_string());
    let repo_ok = payload
        .get(fields::REPOSITORY_ID)
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<RepositoryId>().ok())
        .is_some_and(|r| scope.contains(&r));
    org_ok && repo_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FieldValue;
    use serde_json::json;

    #[test]
    fn extra_filter_cannot_override_tenant() {
        let org = OrganizationId::new();
        assert!(ExtraFilter::new()
            .must(Cond::keyword(fields::ORGANIZATION_ID, org))
            .is_err());
        assert!(ExtraFilter::new()
            .must_not(Cond::any(fields::REPOSITORY_ID, ["x"]))
            .is_err());
        let extra = ExtraFilter::new()
            .must(Cond::keyword(fields::KIND, "doc"))
            .unwrap();
        let repo = RepositoryId::new();
        let f = scoped(&TenantScope::single(org, repo), &extra);
        assert_eq!(
            f.must[0],
            Cond::Match {
                key: fields::ORGANIZATION_ID,
                value: FieldValue::Keyword(org.to_string())
            }
        );
        assert_eq!(
            f.to_json()["must"][1],
            json!({"key": "repository_id", "match": {"any": [repo.to_string()]}})
        );
        assert!(f.should.is_empty());
    }

    #[test]
    fn scope_repositories_are_deduplicated_and_sorted() {
        let (a, b) = (RepositoryId::new(), RepositoryId::new());
        let scope = TenantScope::new(
            OrganizationId::new(),
            NonEmptyVec::new(vec![b, a, b]).unwrap(),
        );
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(scope.repository_ids(), expected.as_slice());
        assert!(NonEmptyVec::<u8>::new(vec![]).is_none());
    }
}
