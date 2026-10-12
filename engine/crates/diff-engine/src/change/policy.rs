//! Classifier policy tables (CHG-005). Defaults are compiled in; `.review/config.yaml`
//! (`classification:`, POL-001) may add patterns but never disable a rule.

use regex::Regex;

/// Longest accepted user pattern (bytes).
pub const MAX_PATTERN_LEN: usize = 256;

/// Patterns and name lists driving the domain classifiers.
#[derive(Debug, Clone)]
pub struct ClassifierPolicy {
    /// Call names that perform an authorization check on their own.
    pub authz_callee_patterns: Vec<Regex>,
    /// Generic call names (`check`) that count as authorization only on an authz-like receiver.
    pub authz_generic_callees: Vec<Regex>,
    /// Receiver fragments that make a generic call an authorization call.
    pub authz_receivers: Vec<Regex>,
    /// Identifiers whose appearance in a condition makes it authorization-relevant.
    pub authz_identifiers: Vec<Regex>,
    /// Guard decorators (last name segment).
    pub guard_decorators: Vec<String>,
    /// Exception classes that signal an authorization failure.
    pub authz_exceptions: Vec<Regex>,
    /// Call names that validate input.
    pub validation_callee_patterns: Vec<Regex>,
    /// Validation decorators (class-validator, pipes).
    pub validation_decorators: Vec<String>,
    /// ORM methods that write.
    pub db_write_methods: Vec<String>,
    /// HTTP route decorators.
    pub http_route_decorators: Vec<String>,
    /// HTTP parameter decorators.
    pub http_param_decorators: Vec<String>,
    /// Response-shaping decorators.
    pub http_response_decorators: Vec<String>,
    /// Call prefixes that only log (an otherwise empty catch that only logs swallows).
    pub swallow_calls: Vec<String>,
}

fn re(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

impl Default for ClassifierPolicy {
    fn default() -> Self {
        Self {
            authz_callee_patterns: [
                r"^(authorize|authorise|can|cannot|hasPermission|hasPermissions|hasRole|hasRoles|hasAccess|isAllowed|isAuthorized|isAuthorised|assertCan|assertPermission|assertAccess|enforce|verifyAccess|checkPermission|checkPermissions|checkAccess|checkRole)$",
            ]
            .iter()
            .filter_map(|p| re(p))
            .collect(),
            authz_generic_callees: [r"^(check|verify|validate|evaluate)$"]
                .iter()
                .filter_map(|p| re(p))
                .collect(),
            authz_receivers: [r"(?i)(perm|auth|acl|polic|guard|access|rbac|role)"]
                .iter()
                .filter_map(|p| re(p))
                .collect(),
            authz_identifiers: [
                r"(?i)^(role|roles|permission|permissions|isadmin|is_admin|admin|scope|scopes|acl|owner|ownerid|owner_id|tenant|tenantid)$",
            ]
            .iter()
            .filter_map(|p| re(p))
            .collect(),
            guard_decorators: names(&[
                "UseGuards",
                "Roles",
                "Role",
                "Public",
                "Permissions",
                "Permission",
                "Auth",
                "Authorize",
                "SkipAuth",
                "AllowAnonymous",
            ]),
            authz_exceptions: [
                r"^(Forbidden\w*|Unauthori[sz]ed\w*|AccessDenied\w*|PermissionDenied\w*)$",
            ]
            .iter()
            .filter_map(|p| re(p))
            .collect(),
            validation_callee_patterns: [
                r"^(validate\w*|validateOrReject|assert[A-Z]\w*|plainToInstance|parse|safeParse|parseAsync|safeParseAsync)$",
            ]
            .iter()
            .filter_map(|p| re(p))
            .collect(),
            validation_decorators: names(&[
                "UsePipes",
                "IsString",
                "IsEmail",
                "IsInt",
                "IsNumber",
                "IsBoolean",
                "IsNotEmpty",
                "IsDefined",
                "IsUUID",
                "IsEnum",
                "IsArray",
                "IsDate",
                "ValidateNested",
                "Min",
                "Max",
                "MinLength",
                "MaxLength",
                "Length",
                "Matches",
            ]),
            db_write_methods: names(&[
                "save",
                "insert",
                "update",
                "delete",
                "remove",
                "upsert",
                "softDelete",
                "softRemove",
                "restore",
                "increment",
                "decrement",
                "destroy",
                "bulkCreate",
                "query",
            ]),
            http_route_decorators: names(&[
                "Get", "Post", "Put", "Patch", "Delete", "All", "Options", "Head",
            ]),
            http_param_decorators: names(&[
                "Body",
                "Query",
                "Param",
                "Headers",
                "Req",
                "Request",
                "Res",
                "UploadedFile",
                "UploadedFiles",
            ]),
            http_response_decorators: names(&[
                "HttpCode",
                "Header",
                "Redirect",
                "Render",
                "SerializeOptions",
                "UseInterceptors",
            ]),
            swallow_calls: names(&["console.", "logger.", "this.logger.", "Logger."]),
        }
    }
}

/// Last `.`-separated segment of a decorator or callee name.
pub fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

impl ClassifierPolicy {
    /// Add user patterns from configuration. Invalid or over-long patterns are rejected with a
    /// message (POL-001 surfaces it at load) and the defaults stay in place.
    pub fn with_extra_authz_callees(mut self, patterns: &[String]) -> Result<Self, String> {
        for p in patterns {
            if p.len() > MAX_PATTERN_LEN {
                return Err(format!("pattern longer than {MAX_PATTERN_LEN} bytes"));
            }
            let anchored = format!("^(?:{p})$");
            let compiled = Regex::new(&anchored).map_err(|e| e.to_string())?;
            self.authz_callee_patterns.push(compiled);
        }
        Ok(self)
    }

    /// Whether a call (`callee_text` such as `this.permissionService.check`) is an
    /// authorization call by name.
    pub fn is_authz_call(&self, callee_text: &str) -> bool {
        let name = last_segment(callee_text);
        if self.authz_callee_patterns.iter().any(|r| r.is_match(name)) {
            return true;
        }
        let receiver = callee_text
            .rsplit_once('.')
            .map_or("", |(receiver, _)| receiver);
        self.authz_generic_callees.iter().any(|r| r.is_match(name))
            && self.authz_receivers.iter().any(|r| r.is_match(receiver))
    }

    /// Whether a thrown class signals an authorization failure.
    pub fn is_authz_exception(&self, class: &str) -> bool {
        let last = last_segment(class);
        self.authz_exceptions.iter().any(|r| r.is_match(last))
    }

    /// Whether an identifier is authorization-relevant (`role`, `permissions`, ...).
    pub fn is_authz_identifier(&self, ident: &str) -> bool {
        let last = last_segment(ident);
        self.authz_identifiers.iter().any(|r| r.is_match(last))
    }

    /// Whether a decorator name is a guard decorator.
    pub fn is_guard_decorator(&self, name: &str) -> bool {
        let last = last_segment(name);
        self.guard_decorators.iter().any(|g| g == last)
            || last.starts_with("Auth")
            || last.starts_with("Authorize")
    }

    /// Whether a call validates input by name.
    pub fn is_validation_call(&self, callee_text: &str) -> bool {
        let name = last_segment(callee_text);
        self.validation_callee_patterns
            .iter()
            .any(|r| r.is_match(name))
    }

    /// Whether a decorator validates input.
    pub fn is_validation_decorator(&self, name: &str) -> bool {
        let last = last_segment(name);
        self.validation_decorators.iter().any(|d| d == last)
    }

    /// Whether a call writes to a database by method name.
    pub fn is_db_write_method(&self, callee_text: &str) -> bool {
        let name = last_segment(callee_text);
        self.db_write_methods.iter().any(|m| m == name)
    }

    /// Whether a decorator declares an HTTP route.
    pub fn is_route_decorator(&self, name: &str) -> bool {
        let last = last_segment(name);
        self.http_route_decorators.iter().any(|d| d == last)
    }

    /// Whether a decorator binds an HTTP parameter.
    pub fn is_param_decorator(&self, name: &str) -> bool {
        let last = last_segment(name);
        self.http_param_decorators.iter().any(|d| d == last)
    }

    /// Whether a decorator shapes the HTTP response.
    pub fn is_response_decorator(&self, name: &str) -> bool {
        let last = last_segment(name);
        self.http_response_decorators.iter().any(|d| d == last)
    }

    /// Whether a call only logs.
    pub fn is_swallow_call(&self, callee_text: &str) -> bool {
        self.swallow_calls.iter().any(|p| callee_text.starts_with(p))
    }

    /// A stable hash of every pattern and list, part of the change-model `input_hash`.
    pub fn policy_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        let mut feed = |s: &str| {
            hasher.update(s.as_bytes());
            hasher.update(&[0]);
        };
        for r in self
            .authz_callee_patterns
            .iter()
            .chain(&self.authz_generic_callees)
            .chain(&self.authz_receivers)
            .chain(&self.authz_identifiers)
            .chain(&self.authz_exceptions)
            .chain(&self.validation_callee_patterns)
        {
            feed(r.as_str());
        }
        for list in [
            &self.guard_decorators,
            &self.validation_decorators,
            &self.db_write_methods,
            &self.http_route_decorators,
            &self.http_param_decorators,
            &self.http_response_decorators,
            &self.swallow_calls,
        ] {
            for s in list {
                feed(s);
            }
            feed("|");
        }
        hasher.finalize().to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_recognise_authorization_calls() {
        let p = ClassifierPolicy::default();
        assert!(p.is_authz_call("this.permissionService.check"));
        assert!(p.is_authz_call("this.auth.authorize"));
        assert!(!p.is_authz_call("this.cart.check"));
        assert!(!p.is_authz_call("this.order.checkout"));
        assert!(p.is_authz_identifier("user.role"));
        assert!(p.is_guard_decorator("UseGuards"));
        assert!(p.is_validation_call("schema.safeParse"));
        assert!(p.is_db_write_method("this.userRepo.save"));
    }
}
