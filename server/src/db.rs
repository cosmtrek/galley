use std::path::Path;

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_drop_comment_kind_scope.sql"),
];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    init(conn)
}

#[cfg(test)]
pub fn open_memory() -> rusqlite::Result<Connection> {
    init(Connection::open_in_memory()?)
}

fn init(mut conn: Connection) -> rusqlite::Result<Connection> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_v1_comments_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute_batch(
            "INSERT INTO reports (id, title, created_at, updated_at) VALUES ('r', 't', 0, 0);
             INSERT INTO comments (id, report_id, kind, scope, status, body, created_version_id, created_at, updated_at)
             VALUES ('c', 'r', 'verify', 'global', 'draft', '全文核实', 'v', 0, 0);",
        )
        .unwrap();
        let conn = init(conn).unwrap();
        let body: String = conn.query_row("SELECT body FROM comments WHERE id = 'c'", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "全文核实");
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Base62 of a 128-bit random number: unguessable tokens for share links and sessions.
pub fn random_token() -> String {
    let mut n: u128 = rand::random();
    let mut s = String::new();
    while n > 0 {
        s.push(BASE62[(n % 62) as usize] as char);
        n /= 62;
    }
    s
}

/// Short readable ids for rows agents refer to in prompts (e.g. `c_k3x9q2`).
pub fn short_id(prefix: &str, len: usize) -> String {
    const ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyz";
    let mut s = String::from(prefix);
    for _ in 0..len {
        let i: usize = rand::random_range(0..ALPHABET.len());
        s.push(ALPHABET[i] as char);
    }
    s
}
