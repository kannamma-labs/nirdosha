fn find_user_email(conn: &Connection, user_id: i64) -> Option<String> {
    let rows = conn.query("SELECT email FROM account WHERE id = $1", &[&user_id]).ok()?;
    for row in rows {
        if let Ok(email) = row.get::<_, String>(0) {
            return Some(email);
        }
    }
    None
}