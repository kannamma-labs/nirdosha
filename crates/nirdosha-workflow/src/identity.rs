//! Operator-owned local directory adapter. It is not a graph/MCP mutation API.
//! Authentication uses random bearer credentials; mappings and roles come from
//! an administered local store, never from approval request fields.
use crate::*;

/// RFC 0021.b's own person-equivalence trust contract version -- the
/// value every installed [`Mapping`]'s own `mapping_schema_version`
/// must match, and the same value the capability manifest's
/// `distinct_person(...)` string reports as its `contract_version`
/// parameter (`Runtime::capabilities`).
pub const MAPPING_SCHEMA_VERSION:&str="nirdosha.runtime.mapping/v1";

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    pub authority_id:String,
    /// The trusted key identifier every installed mapping's own
    /// `authority_key_id` must match -- configuration binds this, the
    /// same way `issuers` is configuration-bound; an assertion cannot
    /// make its own key trusted by merely naming one.
    pub key_id:String,
    pub audience:String, pub tenant:String,
    pub issuers:BTreeSet<String>, pub max_freshness_seconds:u64,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    /// Must equal [`MAPPING_SCHEMA_VERSION`] -- an assertion built
    /// against a different contract version is rejected outright
    /// rather than silently interpreted under this one.
    pub mapping_schema_version:String,
    /// Must equal the configured [`Authority::key_id`].
    pub authority_key_id:String,
    /// Must equal the configured [`Authority::audience`] -- "the
    /// application/runtime identity" this assertion was issued for,
    /// checked so a mapping minted for a different runtime/application
    /// can't be replayed into this one.
    pub audience:String,
    pub issuer:String,pub tenant:String,pub subject:String,pub person:String,
    pub roles:BTreeSet<String>,pub claims:BTreeSet<String>,pub revision:u64,
    pub revocation_revision:u64,pub issued_at:u64,pub expires_at:u64,pub revoked:bool,
    /// "Signature or equivalent authenticated local-store binding"
    /// (RFC 0021.b's own words) -- `None` is a real, accepted value for
    /// this adapter specifically: `install_mapping` is itself the
    /// authenticated local-store binding (a trusted-operator-only Rust
    /// call, never reachable through project MCP, agent patches or
    /// application approval requests -- this module's own top doc
    /// comment). A deployment whose mappings instead come from a
    /// signed remote feed populates this with that real signature; it
    /// is carried through verbatim but not cryptographically verified
    /// by this in-process adapter, since it has no key-verification
    /// infrastructure of its own to check it against.
    pub integrity_proof:Option<String>,
}
impl Mapping {pub fn subject_key(&self)->Result<String>{hash::structured("runtime-subject",&json!([self.issuer,self.tenant,self.subject]))}}
impl Runtime {
    /// Trusted operator boundary. Do not expose this function to application users.
    pub fn install_mapping(&self,mapping:&Mapping)->Result<()> {
        if mapping.mapping_schema_version!=MAPPING_SCHEMA_VERSION{return Err(Error::new("SCHEMA_INVALID","Unsupported mapping_schema_version"))}
        if mapping.authority_key_id!=self.authority.key_id||mapping.audience!=self.authority.audience{return Err(Error::new("UNTRUSTED_IDENTITY","Mapping is not addressed to this authority/audience"))}
        if !self.authority.issuers.contains(&mapping.issuer)||mapping.tenant!=self.authority.tenant||mapping.person.is_empty()||mapping.subject.is_empty(){return Err(Error::new("UNTRUSTED_IDENTITY","Issuer/tenant/person is not configured"))}
        if mapping.expires_at<=mapping.issued_at||mapping.expires_at-mapping.issued_at>self.authority.max_freshness_seconds||mapping.issued_at>now(){return Err(Error::new("IDENTITY_STALE","Mapping freshness exceeds configured policy"))}
        self.tx(||{
            let key=mapping.subject_key()?;
            let old:Option<String>=self.db.query_row("SELECT mapping FROM wf_identity WHERE subject_key=?1",[&key],|r|r.get(0)).optional()?;
            if let Some(old)=old{let old:Mapping=serde_json::from_str(&old)?;if mapping.revision<=old.revision||mapping.revocation_revision<old.revocation_revision{return Err(Error::new("IDENTITY_STALE","Mapping/revocation revision regressed"))}}
            self.db.execute("INSERT INTO wf_identity VALUES(?1,?2) ON CONFLICT(subject_key) DO UPDATE SET mapping=excluded.mapping",params![key,serde_json::to_string(mapping)?])?;
            self.db.execute("INSERT INTO wf_identity_audit(mapping,recorded_at) VALUES(?1,?2)",params![serde_json::to_string(mapping)?,now()])?;Ok(())
        })
    }
    /// The operator/authentication gateway issues this only after authenticating
    /// the federated subject. Tokens are opaque and only their hashes are stored.
    pub fn issue_credential(&self,issuer:&str,tenant:&str,subject:&str,expires:u64)->Result<String>{
        let key=hash::structured("runtime-subject",&json!([issuer,tenant,subject]))?;let mapping=self.mapping(&key)?;
        if expires<=now()||expires>mapping.expires_at{return Err(Error::new("IDENTITY_STALE","Credential must expire within the mapping lease"))}
        let token=id("credential")?;self.db.execute("INSERT INTO wf_credentials VALUES(?1,?2,?3)",params![hash::bytes(token.as_bytes()),key,expires])?;Ok(token)
    }
    pub(crate) fn authenticate(&self,credential:&str)->Result<(String,Mapping)>{
        let row:Option<(String,u64)>=self.db.query_row("SELECT subject_key,expires FROM wf_credentials WHERE token_hash=?1",[hash::bytes(credential.as_bytes())],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((subject,expires))=row else{return Err(Error::new("UNAUTHORIZED","Invalid application credential"))};if expires<=now(){return Err(Error::new("UNAUTHORIZED","Credential expired"))}let mapping=self.mapping(&subject)?;Ok((subject,mapping))
    }
    pub(crate) fn mapping(&self,subject:&str)->Result<Mapping>{
        let value:Option<String>=self.db.query_row("SELECT mapping FROM wf_identity WHERE subject_key=?1",[subject],|r|r.get(0)).optional()?;
        let mapping:Mapping=serde_json::from_str(&value.ok_or_else(||Error::new("UNTRUSTED_IDENTITY","Unknown mapping"))?)?;
        if mapping.revoked||mapping.expires_at<=now()||mapping.issued_at>now()||now().saturating_sub(mapping.issued_at)>self.authority.max_freshness_seconds{return Err(Error::new("IDENTITY_STALE","Mapping revoked, expired or stale"))}
        if !self.authority.issuers.contains(&mapping.issuer)||mapping.tenant!=self.authority.tenant{return Err(Error::new("UNTRUSTED_IDENTITY","Untrusted mapping"))}Ok(mapping)
    }
}
