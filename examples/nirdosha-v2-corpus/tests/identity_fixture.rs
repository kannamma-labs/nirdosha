use nirdosha_v2::{auth_from_identity, mock_issue_token, oidc_validate_token};

nirdosha_rt::roles! {
    Admin = "admin";
}

#[test]
fn fixture_identity_flow_debug() {
    let jwks = r#"{"keys":[{"kid":"key1","kty":"oct","k":"bXktc2VjcmV0LWtleQ"}]}"#;
    let claims = r#"{"roles":["admin"]}"#;
    let token = mock_issue_token("admin-user", "https://mock-idp.local", "features-demo", 1700000000, 999999999, claims, jwks).unwrap();
    println!("TOKEN: {token}");
    let id = oidc_validate_token(&token, "https://mock-idp.local", "features-demo", jwks).unwrap();
    println!("SUBJECT: {:?}", id.subject);
    println!("ROLES: {:?}", id.roles);
    assert!(auth_from_identity(&id).prove::<nirdosha_roles::Admin>().is_ok());
}
