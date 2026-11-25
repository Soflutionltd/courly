//! Blocked senders: mail from these addresses goes to Spam.
//!
//! Addresses are stored lowercased and trimmed so a lookup never misses on
//! case. The list is per account: blocking an address on one account leaves
//! the others untouched.

use super::super::WriteConn;
use super::super::WriterPool;
use super::super::types::DbBlockedSender;
use crate::db::query_as;
use rusqlite::{OptionalExtension, params};

/// Canonical form of an address for storage and comparison.
pub fn normalize_sender_address(address: &str) -> String {
    address.trim().to_lowercase()
}

/// Add `sender_address` to the account's blocked list. Returns `false` when it
/// was already there.
pub fn block_sender(
    conn: &WriteConn<'_>,
    account_id: &str,
    sender_address: &str,
) -> Result<bool, String> {
    let address = normalize_sender_address(sender_address);
    if address.is_empty() {
        return Err("cannot block an empty sender address".to_owned());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO blocked_senders (id, account_id, sender_address)
             VALUES (?1, ?2, ?3)",
            params![id, account_id, address],
        )
        .map_err(|e| format!("block sender: {e}"))?;
    Ok(inserted == 1)
}

/// Remove `sender_address` from the account's blocked list. Returns the row
/// that was removed, so the caller can delete its provider rule, or `None`
/// when the address was not blocked.
pub fn unblock_sender(
    conn: &WriteConn<'_>,
    account_id: &str,
    sender_address: &str,
) -> Result<Option<DbBlockedSender>, String> {
    let address = normalize_sender_address(sender_address);
    let existing = conn
        .query_row(
            "SELECT id, account_id, sender_address, provider_rule_id, created_at
             FROM blocked_senders WHERE account_id = ?1 AND sender_address = ?2",
            params![account_id, address],
            |row| {
                Ok(DbBlockedSender {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    sender_address: row.get("sender_address")?,
                    provider_rule_id: row.get("provider_rule_id")?,
                    created_at: row.get("created_at")?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("read blocked sender: {e}"))?;
    if existing.is_some() {
        conn.execute(
            "DELETE FROM blocked_senders WHERE account_id = ?1 AND sender_address = ?2",
            params![account_id, address],
        )
        .map_err(|e| format!("unblock sender: {e}"))?;
    }
    Ok(existing)
}

/// Record the provider-side rule that enforces this block.
pub fn set_blocked_sender_rule_id(
    conn: &WriteConn<'_>,
    account_id: &str,
    sender_address: &str,
    provider_rule_id: Option<&str>,
) -> Result<(), String> {
    let address = normalize_sender_address(sender_address);
    conn.execute(
        "UPDATE blocked_senders SET provider_rule_id = ?3
         WHERE account_id = ?1 AND sender_address = ?2",
        params![account_id, address, provider_rule_id],
    )
    .map_err(|e| format!("set blocked sender rule: {e}"))?;
    Ok(())
}

/// The account's blocked list, sorted by address.
pub fn list_blocked_senders(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<Vec<DbBlockedSender>, String> {
    query_as::<DbBlockedSender>(
        conn,
        "SELECT id, account_id, sender_address, provider_rule_id, created_at
         FROM blocked_senders WHERE account_id = ?1
         ORDER BY sender_address",
        &[&account_id],
    )
}

/// Which of `sender_addresses` are blocked on the account, normalized.
pub fn blocked_among(
    conn: &WriteConn<'_>,
    account_id: &str,
    sender_addresses: &[String],
) -> Result<Vec<String>, String> {
    let mut blocked = Vec::new();
    for address in sender_addresses {
        let address = normalize_sender_address(address);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blocked_senders WHERE account_id = ?1 AND sender_address = ?2",
                params![account_id, address],
                |row| row.get(0),
            )
            .map_err(|e| format!("blocked lookup: {e}"))?;
        if count > 0 && !blocked.contains(&address) {
            blocked.push(address);
        }
    }
    Ok(blocked)
}

pub async fn db_block_sender(
    db: &WriterPool,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    db.with_write(move |conn| block_sender(conn, &account_id, &sender_address))
        .await
}

pub async fn db_unblock_sender(
    db: &WriterPool,
    account_id: String,
    sender_address: String,
) -> Result<Option<DbBlockedSender>, String> {
    db.with_write(move |conn| unblock_sender(conn, &account_id, &sender_address))
        .await
}

pub async fn db_set_blocked_sender_rule_id(
    db: &WriterPool,
    account_id: String,
    sender_address: String,
    provider_rule_id: Option<String>,
) -> Result<(), String> {
    db.with_write(move |conn| {
        set_blocked_sender_rule_id(
            conn,
            &account_id,
            &sender_address,
            provider_rule_id.as_deref(),
        )
    })
    .await
}

pub async fn db_list_blocked_senders(
    db: &WriterPool,
    account_id: String,
) -> Result<Vec<DbBlockedSender>, String> {
    db.with_write(move |conn| list_blocked_senders(conn, &account_id))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_with_account() -> Connection {
        let raw = Connection::open_in_memory().expect("open");
        crate::db::migrations::run_all(&raw).expect("migrate");
        raw.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'me@example.com', 'imap')",
            [],
        )
        .expect("account");
        raw
    }

    #[test]
    fn blocking_is_case_insensitive_and_idempotent() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        assert!(block_sender(&conn, "acc", " Spammer@Example.COM ").expect("first block"));
        assert!(!block_sender(&conn, "acc", "spammer@example.com").expect("second block"));

        let list = list_blocked_senders(&conn, "acc").expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sender_address, "spammer@example.com");

        let hits = blocked_among(
            &conn,
            "acc",
            &[
                "SPAMMER@example.com".to_owned(),
                "friend@example.com".to_owned(),
            ],
        )
        .expect("lookup");
        assert_eq!(hits, vec!["spammer@example.com".to_owned()]);
    }

    #[test]
    fn unblocking_returns_the_rule_to_remove() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        block_sender(&conn, "acc", "spammer@example.com").expect("block");
        set_blocked_sender_rule_id(&conn, "acc", "spammer@example.com", Some("rule-1"))
            .expect("set rule");

        let removed = unblock_sender(&conn, "acc", "Spammer@example.com")
            .expect("unblock")
            .expect("was blocked");
        assert_eq!(removed.provider_rule_id.as_deref(), Some("rule-1"));
        assert!(list_blocked_senders(&conn, "acc").expect("list").is_empty());
        assert!(
            unblock_sender(&conn, "acc", "spammer@example.com")
                .expect("second unblock")
                .is_none()
        );
    }

    #[test]
    fn an_empty_address_is_refused() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        assert!(block_sender(&conn, "acc", "   ").is_err());
    }

    #[test]
    fn deleting_the_account_drops_its_blocks() {
        let raw = open_with_account();
        raw.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        block_sender(&WriteConn::from_raw(&raw), "acc", "spammer@example.com").expect("block");
        raw.execute("DELETE FROM accounts WHERE id = 'acc'", [])
            .expect("delete account");
        let count: i64 = raw
            .query_row("SELECT COUNT(*) FROM blocked_senders", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }
}
