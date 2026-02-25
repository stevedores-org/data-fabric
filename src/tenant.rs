use worker::*;

const TENANT_ID_HEADER: &str = "x-tenant-id";
const TENANT_ROLE_HEADER: &str = "x-tenant-role";
const TENANT_FED_HEADER: &str = "x-tenant-federation";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantRole {
    Viewer,
    Builder,
    Admin,
}

#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: String,
    pub role: TenantRole,
    #[allow(dead_code)]
    pub federation_allowed: bool,
}

impl TenantContext {
    pub fn r2_prefix(&self) -> String {
        format!("tenants/{}/", self.tenant_id)
    }
}

pub fn tenant_from_request(req: &Request) -> Result<TenantContext> {
    let headers = req.headers();
    let tenant_id = headers
        .get(TENANT_ID_HEADER)?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::RustError("missing x-tenant-id".to_string()))?;

    let role = parse_role(
        headers
            .get(TENANT_ROLE_HEADER)?
            .unwrap_or_else(|| "viewer".into())
            .as_str(),
    )?;
    let federation_allowed = headers
        .get(TENANT_FED_HEADER)?
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    Ok(TenantContext {
        tenant_id,
        role,
        federation_allowed,
    })
}

pub fn authorize(ctx: &TenantContext, method: Method, path: &str) -> Result<()> {
    let is_read = matches!(method, Method::Get | Method::Head | Method::Options);

    // Tenant provisioning and policy activation are admin-only.
    if path.starts_with("/v1/tenants/") || path.starts_with("/v1/policies/activate") {
        if ctx.role != TenantRole::Admin {
            return Err(Error::RustError("admin role required".to_string()));
        }
        return Ok(());
    }

    // Policy rules writes are admin-only.
    if path.starts_with("/v1/policies/rules") && !is_read && ctx.role != TenantRole::Admin {
        return Err(Error::RustError(
            "admin role required for policy rule mutation".to_string(),
        ));
    }

    // Viewer is read-only everywhere else.
    if ctx.role == TenantRole::Viewer && !is_read {
        return Err(Error::RustError("viewer role is read-only".to_string()));
    }

    Ok(())
}

fn parse_role(v: &str) -> Result<TenantRole> {
    match v.to_ascii_lowercase().as_str() {
        "viewer" => Ok(TenantRole::Viewer),
        "builder" => Ok(TenantRole::Builder),
        "admin" => Ok(TenantRole::Admin),
        _ => Err(Error::RustError(
            "invalid x-tenant-role (expected viewer|builder|admin)".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(role: TenantRole) -> TenantContext {
        TenantContext {
            tenant_id: "tenant-42".to_string(),
            role,
            federation_allowed: false,
        }
    }

    // ── parse_role ─────────────────────────────────────────────

    #[test]
    fn parse_role_valid_values() {
        assert_eq!(parse_role("viewer").unwrap(), TenantRole::Viewer);
        assert_eq!(parse_role("builder").unwrap(), TenantRole::Builder);
        assert_eq!(parse_role("admin").unwrap(), TenantRole::Admin);
    }

    #[test]
    fn parse_role_case_insensitive() {
        assert_eq!(parse_role("VIEWER").unwrap(), TenantRole::Viewer);
        assert_eq!(parse_role("Builder").unwrap(), TenantRole::Builder);
        assert_eq!(parse_role("ADMIN").unwrap(), TenantRole::Admin);
    }

    #[test]
    fn parse_role_invalid_returns_error() {
        assert!(parse_role("superadmin").is_err());
        assert!(parse_role("").is_err());
        assert!(parse_role("root").is_err());
    }

    // ── TenantContext::r2_prefix ───────────────────────────────

    #[test]
    fn r2_prefix_format() {
        let tc = ctx(TenantRole::Viewer);
        assert_eq!(tc.r2_prefix(), "tenants/tenant-42/");
    }

    #[test]
    fn r2_prefix_different_tenant() {
        let tc = TenantContext {
            tenant_id: "org-abc".to_string(),
            role: TenantRole::Admin,
            federation_allowed: true,
        };
        assert_eq!(tc.r2_prefix(), "tenants/org-abc/");
    }

    // ── authorize: admin paths ─────────────────────────────────

    #[test]
    fn admin_can_access_tenant_provisioning() {
        assert!(authorize(&ctx(TenantRole::Admin), Method::Post, "/v1/tenants/provision").is_ok());
    }

    #[test]
    fn viewer_cannot_access_tenant_provisioning() {
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Post, "/v1/tenants/provision").is_err());
    }

    #[test]
    fn viewer_cannot_get_admin_only_paths() {
        // Admin-only paths deny all non-admin roles regardless of HTTP method
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Get, "/v1/tenants/list").is_err());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Get, "/v1/policies/activate/v1").is_err());
    }

    #[test]
    fn builder_cannot_get_admin_only_paths() {
        assert!(authorize(&ctx(TenantRole::Builder), Method::Get, "/v1/tenants/list").is_err());
        assert!(authorize(&ctx(TenantRole::Builder), Method::Get, "/v1/policies/activate/v1").is_err());
    }

    #[test]
    fn admin_can_get_admin_only_paths() {
        assert!(authorize(&ctx(TenantRole::Admin), Method::Get, "/v1/tenants/list").is_ok());
        assert!(authorize(&ctx(TenantRole::Admin), Method::Get, "/v1/policies/activate/v1").is_ok());
    }

    #[test]
    fn builder_cannot_access_tenant_provisioning() {
        assert!(authorize(&ctx(TenantRole::Builder), Method::Post, "/v1/tenants/provision").is_err());
    }

    #[test]
    fn admin_can_activate_policy() {
        assert!(authorize(&ctx(TenantRole::Admin), Method::Post, "/v1/policies/activate/v1").is_ok());
    }

    #[test]
    fn viewer_cannot_activate_policy() {
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Post, "/v1/policies/activate/v1").is_err());
    }

    // ── authorize: policy rules paths ──────────────────────────

    #[test]
    fn admin_can_write_policy_rules() {
        assert!(authorize(&ctx(TenantRole::Admin), Method::Post, "/v1/policies/rules").is_ok());
    }

    #[test]
    fn builder_cannot_write_policy_rules() {
        assert!(authorize(&ctx(TenantRole::Builder), Method::Post, "/v1/policies/rules").is_err());
    }

    #[test]
    fn viewer_can_read_policy_rules() {
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Get, "/v1/policies/rules").is_ok());
    }

    // ── authorize: viewer read-only ────────────────────────────

    #[test]
    fn viewer_can_read_general_paths() {
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Get, "/v1/artifacts").is_ok());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Head, "/v1/artifacts").is_ok());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Options, "/v1/anything").is_ok());
    }

    #[test]
    fn viewer_cannot_write_general_paths() {
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Post, "/v1/artifacts").is_err());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Put, "/v1/artifacts").is_err());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Delete, "/v1/artifacts").is_err());
        assert!(authorize(&ctx(TenantRole::Viewer), Method::Patch, "/v1/artifacts").is_err());
    }

    // ── authorize: builder can write ───────────────────────────

    #[test]
    fn builder_can_write_general_paths() {
        assert!(authorize(&ctx(TenantRole::Builder), Method::Post, "/v1/artifacts").is_ok());
        assert!(authorize(&ctx(TenantRole::Builder), Method::Put, "/v1/artifacts").is_ok());
        assert!(authorize(&ctx(TenantRole::Builder), Method::Delete, "/v1/artifacts").is_ok());
    }

    #[test]
    fn builder_can_read_general_paths() {
        assert!(authorize(&ctx(TenantRole::Builder), Method::Get, "/v1/artifacts").is_ok());
    }
}
