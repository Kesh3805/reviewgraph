//! Shared fixtures for reviewer tests: a small auth-bypass context and a golden model output.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::Duration;

use model_gateway::{CallBudget, PrivacyClass, RiskBand, TenantScope, TraceContext};
use review_core::ids::{OrganizationId, RepositoryId};
use reviewers::context::{
    ChangeSummary, ChangedSymbol, ContextSnapshot, Hunk, NeighborEdge, NeighborNode, RiskSignal,
    RuleItem, TestItem,
};
use reviewers::{FocusProfile, ReviewRequest, RiskAssessment};
use serde_json::{json, Value};

pub const AUTHORIZE: &str = "ts:src/auth/auth.service#AuthService.authorize/method";
pub const UPDATE_USER: &str = "ts:src/admin/admin.service#AdminService.updateUser/method";
pub const CONTROLLER: &str = "ts:src/admin/admin.controller#AdminController.update/method";

pub fn auth_bypass_context() -> ContextSnapshot {
    ContextSnapshot {
        change_summary: ChangeSummary {
            files_changed: 1,
            intent: Some("refactor".into()),
            change_classes: vec!["call_removed".into()],
        },
        changed_symbols: vec![ChangedSymbol {
            symbol_id: AUTHORIZE.into(),
            kind: "method".into(),
            path: "src/auth/auth.service.ts".into(),
            range: [12, 17],
            base_range: Some([12, 18]),
            file_lines: Some(40),
            change_classes: vec!["call_removed".into()],
            signature_base: Some("authorize(user: User, resource: Resource): Promise<void>".into()),
            signature_head: Some("authorize(user: User, resource: Resource): Promise<void>".into()),
            body_base: Some(
                "async authorize(user: User, resource: Resource): Promise<void> {\n  if (!user) throw new UnauthorizedException();\n  await this.permissions.check(user, resource);\n}"
                    .into(),
            ),
            body_head: Some(
                "async authorize(user: User, resource: Resource): Promise<void> {\n  if (!user) throw new UnauthorizedException();\n}"
                    .into(),
            ),
            hunks: vec![Hunk {
                old: [14, 14],
                new: [13, 13],
            }],
            generated: false,
        }],
        nodes: vec![
            NeighborNode {
                symbol_id: UPDATE_USER.into(),
                kind: "method".into(),
                path: "src/admin/admin.service.ts".into(),
                range: [40, 61],
                file_lines: Some(80),
                distance: 1,
                excerpt: "await this.auth.authorize(actor, target);\nreturn this.users.update(target.id, patch);"
                    .into(),
            },
            NeighborNode {
                symbol_id: CONTROLLER.into(),
                kind: "method".into(),
                path: "src/admin/admin.controller.ts".into(),
                range: [20, 30],
                file_lines: Some(50),
                distance: 2,
                excerpt: "@Patch(':id')\nupdate(@Req() req, @Param('id') id: string, @Body() patch: UpdateUserDto)"
                    .into(),
            },
        ],
        edges: vec![
            NeighborEdge {
                from: UPDATE_USER.into(),
                to: AUTHORIZE.into(),
                kind: "CALLS".into(),
                confidence: 0.95,
            },
            NeighborEdge {
                from: CONTROLLER.into(),
                to: UPDATE_USER.into(),
                kind: "CALLS".into(),
                confidence: 0.9,
            },
        ],
        tests: vec![TestItem {
            test_id: "test:src/auth/auth.service.spec.ts#authorize".into(),
            path: "src/auth/auth.service.spec.ts".into(),
            covers: vec![AUTHORIZE.into()],
        }],
        rules: vec![RuleItem {
            rule_id: "rule:admin-permission-check".into(),
            text: "Every admin operation must call PermissionService.check.".into(),
            source: "policy".into(),
        }],
        diagnostics: Vec::new(),
        config_items: Vec::new(),
        risk_signals: vec![RiskSignal {
            signal: "authorization_logic_changed".into(),
            weight: 0.9,
        }],
    }
}

/// A valid `reviewer_output.v1` answer for [`auth_bypass_context`].
pub fn golden_output() -> Value {
    json!({
        "no_findings_reason": null,
        "findings": [{
            "category": "security",
            "title": "authorize no longer checks permissions",
            "claim": "S1 no longer calls PermissionService.check, so N1 updates users without a permission check",
            "description": "The permission check was removed from AuthService.authorize. AdminService.updateUser relies on it before writing.",
            "severity": "high",
            "anchor": {"ref": "S1", "side": "head", "start_line": 12, "end_line": 15},
            "affected_refs": ["S1", "N1", "T1"],
            "evidence": [
                {"kind": "changed_source", "ref": "S1", "side": "base", "start_line": 14, "end_line": 14,
                 "quote": "await this.permissions.check(user, resource);",
                 "explanation": "The base version enforced the permission check."},
                {"kind": "caller_path", "ref": "N1", "side": "head", "start_line": 41, "end_line": 41,
                 "quote": "await this.auth.authorize(actor, target);",
                 "explanation": "updateUser relies on authorize before writing."}
            ],
            "claimed_relations": [
                {"from": "N1", "relation": "calls", "to": "S1"},
                {"from": "N2", "relation": "reaches_endpoint", "to": "N1"}
            ],
            "predicate": {"kind": "call_removed", "subject": "S1",
                          "params": [{"name": "callee", "value": "PermissionService.check"}]},
            "corrective_direction": "Restore the permission check in authorize.",
            "self_confidence": 0.8
        }]
    })
}

pub fn tenant() -> TenantScope {
    TenantScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

pub fn risk() -> RiskAssessment {
    RiskAssessment {
        level: RiskBand::High,
        modules_touched: 1,
        signals: vec!["authorization_logic_changed".into()],
    }
}

pub fn request<'a>(
    cx: &'a ContextSnapshot,
    risk: &'a RiskAssessment,
    focus: &'a [FocusProfile],
    trace: &'a TraceContext,
) -> ReviewRequest<'a> {
    ReviewRequest {
        context: cx,
        risk,
        cluster_id: "c1",
        focus,
        tenant: tenant(),
        budget: CallBudget::within(Duration::from_secs(60)),
        trace,
        privacy: PrivacyClass::Standard,
    }
}
