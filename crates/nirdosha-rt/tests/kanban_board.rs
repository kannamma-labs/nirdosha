//! `kanban_board!` end to end: the board is presentation only, and a
//! card move goes through an existing `#[contract(requires(role))]`-
//! gated function wired to a route by hand -- the same pattern the
//! flagship demo uses for approve/disapprove -- proving the board never
//! regenerates its own mutation/authorization logic.

use nirdosha_rt::{Auth, PathParams, Request, Response, Router};
use std::collections::HashMap;
use std::sync::Mutex;

nirdosha_rt::roles! {
    Lead = "lead";
}

#[derive(Clone, Default, serde::Serialize)]
struct Ticket {
    id: i64,
    title: String,
    status: String,
}

fn ticket_store() -> &'static Mutex<HashMap<i64, Ticket>> {
    static STORE: std::sync::OnceLock<Mutex<HashMap<i64, Ticket>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert(1, Ticket { id: 1, title: "Fix bug".to_string(), status: "backlog".to_string() });
        m.insert(2, Ticket { id: 2, title: "Ship feature".to_string(), status: "in_progress".to_string() });
        Mutex::new(m)
    })
}

#[nirdosha_rt::contract(requires(role = "lead"))]
fn move_ticket(id: i64, to: &str) -> Result<Ticket, &'static str> {
    let mut store = ticket_store().lock().unwrap();
    let ticket = store.get_mut(&id).ok_or("not found")?;
    ticket.status = to.to_string();
    Ok(ticket.clone())
}

nirdosha_rt::kanban_board! {
    mount: mount_ticket_board,
    entity: Ticket,
    store: ticket_store,
    path: "/board",
    access: public,
    title_field: title,
    column_field: status,
    columns: [ "backlog", "in_progress", "done" ],
    move_path: "/tickets/{id}/move/{to}",
}

fn router() -> Router {
    let router = Router::new(|req: &Request| {
        let roles: Vec<&str> = req.header("x-roles").map(|s| s.split(',').collect()).unwrap_or_default();
        Auth::login("test", &roles)
    });
    let router = mount_ticket_board(router);
    router.post_gated::<nirdosha_roles::Lead>("/tickets/{id}/move/{to}", "Move ticket", |_req: &Request, params: &PathParams, proof| {
        let id: i64 = match params.get("id").and_then(|s| s.parse().ok()) {
            Some(id) => id,
            None => return Response::bad_request("id must be an integer"),
        };
        let to = params.get("to").unwrap_or_default();
        match move_ticket(proof, id, to) {
            Ok(_) => Response::no_content(),
            Err(_) => Response::not_found(),
        }
    })
}

fn req(method: &str, path: &str, roles: Option<&str>) -> Request {
    let mut headers = HashMap::new();
    if let Some(r) = roles {
        headers.insert("x-roles".to_string(), r.to_string());
    }
    Request { method: method.into(), path: path.into(), headers, body: String::new() }
}

fn dispatch(method: &str, path: &str, roles: Option<&str>) -> Response {
    router().dispatch(&req(method, path, roles))
}

#[test]
fn board_renders_cards_in_their_current_columns() {
    let resp = dispatch("GET", "/board", None);
    assert_eq!(resp.status, 200);
    assert!(resp.body.contains("data-column=\"backlog\""));
    assert!(resp.body.contains("Fix bug"));
    assert!(resp.body.contains("Ship feature"));
    assert!(resp.body.contains("/board/board.js"));
}

#[test]
fn board_js_embeds_the_apps_own_move_path_template() {
    let resp = dispatch("GET", "/board/board.js", None);
    assert_eq!(resp.status, 200);
    assert!(resp.content_type.starts_with("text/javascript"));
    assert!(resp.body.contains("/tickets/{id}/move/{to}"));
    assert!(resp.body.contains("dragstart"));
    assert!(resp.body.contains("drop"));
}

#[test]
fn moving_a_card_goes_through_the_apps_own_gated_endpoint_not_the_board() {
    // Anonymous can view the board (it's public) but can't move a card --
    // the move endpoint is the app's own categorical_actions!-gated
    // function, wholly independent of the board's own (public) access.
    let denied = dispatch("POST", "/tickets/1/move/done", None);
    assert_eq!(denied.status, 403);
    assert_eq!(ticket_store().lock().unwrap()[&1].status, "backlog");

    let ok = dispatch("POST", "/tickets/1/move/done", Some("lead"));
    assert_eq!(ok.status, 204);
    assert_eq!(ticket_store().lock().unwrap()[&1].status, "done");

    // The board reflects the move on next render.
    let after = dispatch("GET", "/board", None);
    assert!(after.body.contains("data-column=\"done\""));
}
