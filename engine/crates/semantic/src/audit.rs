//! Tenant audit layer over the Qdrant client (SEM-005, SEC-002). Test builds only
//! (`feature = "audit"`): it records every request body and checks that each read, delete and
//! payload update carries the organization and repository conditions in `filter.must`, and that
//! every upserted point carries both tenant keys.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::filter::fields;

/// Operations whose body must carry a tenant filter.
pub const FILTERED_OPS: [&str; 5] = ["search", "scroll", "count", "delete", "set_payload"];

/// One recorded request.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRecord {
    pub op: &'static str,
    pub collection: String,
    pub body: Value,
}

/// Records requests and tenant-filter violations.
#[derive(Debug, Default)]
pub struct TenantAudit {
    records: Mutex<Vec<AuditRecord>>,
    violations: Mutex<Vec<String>>,
    strict: bool,
}

fn has_cond(must: &[Value], key: &str, any_allowed: bool) -> bool {
    must.iter().any(|c| {
        if c.get("key").and_then(Value::as_str) != Some(key) {
            return false;
        }
        let m = &c["match"];
        let single = m.get("value").and_then(Value::as_str).is_some();
        let any = any_allowed
            && m.get("any")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_string));
        single || any
    })
}

fn mentions_tenant_key(conds: Option<&Value>) -> bool {
    conds.and_then(Value::as_array).is_some_and(|a| {
        a.iter().any(|c| {
            c.get("key")
                .and_then(Value::as_str)
                .is_some_and(|k| fields::TENANT_KEYS.contains(&k))
        })
    })
}

/// Checks one request body. `Err` describes the violation (values elided).
pub fn check_request(op: &str, body: &Value) -> Result<(), String> {
    if FILTERED_OPS.contains(&op) {
        let filter = &body["filter"];
        let must = filter
            .get("must")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !has_cond(must, fields::ORGANIZATION_ID, false) {
            return Err(format!("{op}: filter.must lacks organization_id"));
        }
        if !has_cond(must, fields::REPOSITORY_ID, true) {
            return Err(format!("{op}: filter.must lacks repository_id"));
        }
        if mentions_tenant_key(filter.get("should")) || mentions_tenant_key(filter.get("must_not"))
        {
            return Err(format!("{op}: tenant key in should/must_not"));
        }
    } else if op == "upsert" {
        let points = body
            .get("points")
            .and_then(Value::as_array)
            .ok_or_else(|| "upsert: no points".to_owned())?;
        for (i, p) in points.iter().enumerate() {
            let payload = &p["payload"];
            for key in fields::TENANT_KEYS {
                if payload.get(key).and_then(Value::as_str).is_none() {
                    return Err(format!("upsert: point {i} payload lacks {key}"));
                }
            }
        }
    }
    Ok(())
}

impl TenantAudit {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Panics on the first violation (for suites that want the offending stack).
    pub fn strict() -> Arc<Self> {
        Arc::new(Self {
            strict: true,
            ..Self::default()
        })
    }

    /// Called by the client before each request is sent.
    #[allow(clippy::panic)]
    pub fn observe(&self, op: &'static str, collection: &str, body: &Value) {
        if let Ok(mut r) = self.records.lock() {
            r.push(AuditRecord {
                op,
                collection: collection.to_owned(),
                body: body.clone(),
            });
        }
        if let Err(v) = check_request(op, body) {
            if self.strict {
                panic!("tenant audit violation: {v}");
            }
            if let Ok(mut vs) = self.violations.lock() {
                vs.push(v);
            }
        }
    }

    pub fn records(&self) -> Vec<AuditRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn violations(&self) -> Vec<String> {
        self.violations
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// Number of recorded requests of `op`.
    pub fn count(&self, op: &str) -> usize {
        self.records().iter().filter(|r| r.op == op).count()
    }

    /// Panics with every violation, if any.
    #[allow(clippy::panic)]
    pub fn assert_clean(&self) {
        let v = self.violations();
        if !v.is_empty() {
            panic!("tenant audit found {} violation(s): {v:#?}", v.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unscoped_search_is_a_violation() {
        assert!(check_request("search", &json!({"filter": {}})).is_err());
        assert!(check_request(
            "search",
            &json!({"filter": {"must": [{"key": "organization_id", "match": {"value": "o"}}]}})
        )
        .is_err());
        assert!(check_request(
            "search",
            &json!({"filter": {"must": [
                {"key": "organization_id", "match": {"value": "o"}},
                {"key": "repository_id", "match": {"any": ["r"]}}
            ], "should": [{"key": "organization_id", "match": {"value": "x"}}]}})
        )
        .is_err());
        assert!(check_request(
            "delete",
            &json!({"filter": {"must": [
                {"key": "organization_id", "match": {"value": "o"}},
                {"key": "repository_id", "match": {"any": ["r"]}}
            ]}})
        )
        .is_ok());
        assert!(check_request("upsert", &json!({"points": [{"payload": {}}]})).is_err());
    }
}
