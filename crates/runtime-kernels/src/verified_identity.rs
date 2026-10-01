//! Safe, authority-bound identity using the same JWT validator as the native
//! C ABI. Configuration and the clock belong to the trusted host, not a request.
use std::sync::Arc;

#[derive(Clone)]
pub struct Authority(Arc<Configuration>);
struct Configuration {
    issuer: String,
    audience: String,
    jwks: String,
}

/// Cannot be constructed from caller-asserted roles or claims.
pub struct Identity {
    authority: Arc<Configuration>,
    claims: super::VerifiedClaims,
    roles: Vec<String>,
    not_before: i64,
}

/// A grant is scoped to this identity and its authorizing authority.
pub struct Grant<'a> {
    identity: &'a Identity,
    role: String,
}
impl Grant<'_> {
    pub fn subject(&self) -> &str {
        &self.identity.claims.subject
    }
    pub fn role(&self) -> &str {
        &self.role
    }
}

impl Authority {
    /// Called at trusted application startup with configured issuer keys.
    /// Never populate these arguments from an untrusted request/token.
    pub fn new(issuer: String, audience: String, jwks: String) -> Result<Self, String> {
        if issuer.is_empty() || audience.is_empty() {
            return Err("issuer and audience are required".into());
        }
        let keys: super::RawJwks =
            serde_json::from_str(&jwks).map_err(|e| format!("invalid JWKS: {e}"))?;
        if keys.keys.is_empty() {
            return Err("JWKS must contain keys".into());
        }
        let mut ids = std::collections::HashSet::new();
        for key in &keys.keys {
            if key.kid.is_empty() || !ids.insert(&key.kid) {
                return Err("empty or duplicate key id".into());
            }
            super::decoding_key_for(key)?;
        }
        Ok(Self(Arc::new(Configuration {
            issuer,
            audience,
            jwks,
        })))
    }

    pub fn verify(&self, token: &str, now: i64) -> Result<Identity, String> {
        let claims = super::validate_oidc_token_inner(
            token,
            &self.0.issuer,
            &self.0.audience,
            &self.0.jwks,
        )?;
        let json: serde_json::Value =
            serde_json::from_str(&claims.claims_json).map_err(|e| e.to_string())?;
        let not_before = match json.get("nbf") {
            None => 0,
            Some(value) => value.as_i64().ok_or("nbf must be an integer")?,
        };
        let roles = match json.get("roles") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("roles must be an array")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or("roles must contain strings")
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let identity = Identity {
            authority: self.0.clone(),
            claims,
            roles,
            not_before,
        };
        self.check(&identity, now)?;
        Ok(identity)
    }

    fn check(&self, identity: &Identity, now: i64) -> Result<(), String> {
        if !Arc::ptr_eq(&self.0, &identity.authority) {
            return Err("identity belongs to another authority".into());
        }
        if identity.claims.subject.is_empty() {
            return Err("subject is required".into());
        }
        if now < 0 || identity.claims.issued_at > now || identity.not_before > now {
            return Err("identity is not yet valid".into());
        }
        if identity.claims.expires_at <= now {
            return Err("identity has expired".into());
        }
        Ok(())
    }

    /// Rechecks expiry on every authorization, including identities retained
    /// between requests. The returned grant cannot outlive that identity.
    pub fn authorize<'a>(
        &self,
        identity: &'a Identity,
        role: &str,
        now: i64,
    ) -> Result<Grant<'a>, String> {
        self.check(identity, now)?;
        if !identity.roles.iter().any(|r| r == role) {
            return Err(format!("missing role `{role}`"));
        }
        Ok(Grant {
            identity,
            role: role.to_owned(),
        })
    }
}
