//! The accounts and the inbox, read straight from the device's Courly
//! database with the desktop's own queries. The engine writes the
//! database; these reads open their own connection, which SQLite's WAL
//! mode lets run alongside the engine's writes.

use std::path::Path;

use db::db::ReadConn;
use db::db::queries_extra::{get_all_accounts_sync, get_threads_scoped};
use db::db::types::AccountScope;

/// One account as the phone lists it.
pub struct AccountSummary {
    pub id: String,
    pub email: String,
    /// The name the user gave the account, else its domain.
    pub name: String,
    /// The account's colour, as `#rrggbb`, when it has one.
    pub color: Option<String>,
}

/// One conversation as the inbox list shows it.
pub struct ThreadSummary {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub snippet: String,
    /// The latest sender's name, else their address.
    pub sender: String,
    /// When the latest message arrived, in milliseconds since the epoch.
    pub date_ms: Option<i64>,
    pub is_read: bool,
    pub has_attachments: bool,
    pub is_pinned: bool,
}

fn open(data_dir: &str) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(
        Path::new(data_dir).join(db::db::DB_FILE_NAME),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("open database: {e}"))
}

/// Every account on the device, in the order the user set.
pub fn list_accounts(data_dir: String) -> Result<Vec<AccountSummary>, String> {
    let conn = open(&data_dir)?;
    let accounts = get_all_accounts_sync(&ReadConn::from_raw(&conn))?;
    Ok(accounts
        .into_iter()
        .map(|account| AccountSummary {
            name: account
                .account_name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| {
                    account
                        .email
                        .split('@')
                        .nth(1)
                        .unwrap_or(&account.email)
                        .to_owned()
                }),
            id: account.id,
            email: account.email,
            color: account.account_color,
        })
        .collect())
}

/// The newest conversations of the inbox: of one account, or of every
/// account when `account_id` is `None`, pinned ones first.
pub fn inbox_threads(
    data_dir: String,
    account_id: Option<String>,
    limit: u32,
) -> Result<Vec<ThreadSummary>, String> {
    let conn = open(&data_dir)?;
    let scope = match account_id {
        Some(id) => AccountScope::Single(id),
        None => AccountScope::All,
    };
    let threads = get_threads_scoped(
        &ReadConn::from_raw(&conn),
        &scope,
        Some("INBOX"),
        Some(i64::from(limit)),
        Some(0),
    )?;
    Ok(threads
        .into_iter()
        .map(|thread| ThreadSummary {
            sender: thread
                .from_name
                .filter(|name| !name.trim().is_empty())
                .or(thread.from_address)
                .unwrap_or_default(),
            id: thread.id,
            account_id: thread.account_id,
            subject: thread.subject.unwrap_or_default(),
            snippet: thread.snippet.unwrap_or_default(),
            date_ms: thread.last_message_at,
            is_read: thread.is_read,
            has_attachments: thread.has_attachments,
            is_pinned: thread.is_pinned,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_device_has_no_account_and_an_empty_inbox() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().into_owned();
        super::super::database::open_courly_database(path.clone()).expect("migrate");
        assert!(list_accounts(path.clone()).expect("accounts").is_empty());
        assert!(inbox_threads(path, None, 50).expect("inbox").is_empty());
    }
}
