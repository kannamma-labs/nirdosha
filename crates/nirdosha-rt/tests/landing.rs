//! `landing! { .. }` (issue #70) end to end: first-match-wins role
//! rules pick a post-login redirect target, `default` is the required
//! catch-all, matching `.nir`'s own `LandingDecl`/`typeck::check_landing`
//! semantics (`crates/compiler/src/ast.rs`'s `LandingDecl` doc comment).

nirdosha_rt::landing! {
    role("admin") -> "/admin",
    role("hr_staff") -> "/hr",
    default -> "/home",
}

#[test]
fn the_first_matching_role_rule_wins() {
    let admin = nirdosha_rt::Auth::login("priya", &["admin", "hr_staff"]);
    assert_eq!(landing_path(&admin), "/admin");
}

#[test]
fn a_later_role_rule_matches_when_earlier_ones_do_not() {
    let hr = nirdosha_rt::Auth::login("sita", &["hr_staff"]);
    assert_eq!(landing_path(&hr), "/hr");
}

#[test]
fn no_matching_role_falls_back_to_default() {
    let nobody = nirdosha_rt::Auth::login("ravana", &["janitor"]);
    assert_eq!(landing_path(&nobody), "/home");
}
