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
