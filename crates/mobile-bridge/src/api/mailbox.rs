//! The accounts, their folders and the conversations in them, read straight from the device's Courly
//! database with the desktop's own queries. The engine writes the
//! database; these reads open their own connection, which SQLite's WAL
//! mode lets run alongside the engine's writes.

use std::path::Path;

use db::db::ReadConn;
use db::db::queries_extra::{
    get_all_accounts_sync, get_threads_scoped, get_unread_counts_by_folder,
    get_unread_counts_by_folder_and_account,
};
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

/// A system folder the phone can open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailFolder {
    Inbox,
    Sent,
    Archive,
    Trash,
    Spam,
}

impl MailFolder {
    const ALL: [Self; 5] = [
        Self::Inbox,
        Self::Sent,
        Self::Archive,
        Self::Trash,
        Self::Spam,
    ];

    /// The folder's id in the database, the same on every provider.
    fn id(self) -> &'static str {
        match self {
            Self::Inbox => "INBOX",
            Self::Sent => "SENT",
            Self::Archive => "archive",
            Self::Trash => "TRASH",
            Self::Spam => "SPAM",
        }
    }
}

/// How many unread conversations a folder holds.
pub struct FolderUnread {
    /// The account the folder belongs to, or `None` for the inbox of every
    /// account together.
    pub account_id: Option<String>,
    pub folder: MailFolder,
    pub unread: u32,
}

/// One conversation as a folder's list shows it.
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

/// The unread count of every folder that has unread mail: each account's
/// folders, and the inbox of every account together, which is wider than
/// the sum of the inboxes (it also takes mail filed in custom folders).
pub fn unread_counts(data_dir: String) -> Result<Vec<FolderUnread>, String> {
    let conn = open(&data_dir)?;
    let conn = ReadConn::from_raw(&conn);
    let folder_of = |id: &str| MailFolder::ALL.into_iter().find(|folder| folder.id() == id);
    let unread_of = |count: i64| u32::try_from(count).unwrap_or(u32::MAX);
    let mut counts: Vec<FolderUnread> =
        get_unread_counts_by_folder_and_account(&conn, &AccountScope::All)?
            .into_iter()
            .filter_map(|count| {
                Some(FolderUnread {
                    folder: folder_of(&count.folder_id)?,
                    account_id: Some(count.account_id),
                    unread: unread_of(count.unread_count.as_i64()),
                })
            })
            .collect();
    counts.extend(
        get_unread_counts_by_folder(&conn, &AccountScope::All)?
            .into_iter()
            .filter(|count| count.folder_id == MailFolder::Inbox.id())
            .map(|count| FolderUnread {
                folder: MailFolder::Inbox,
                account_id: None,
                unread: unread_of(count.unread_count.as_i64()),
            }),
    );
    counts.retain(|count| count.unread > 0);
    Ok(counts)
}

/// The newest conversations of a folder: of one account, or of every
/// account when `account_id` is `None`. The inbox lists pinned ones first.
pub fn folder_threads(
    data_dir: String,
    account_id: Option<String>,
    folder: MailFolder,
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
        Some(folder.id()),
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
    fn a_fresh_device_has_no_account_no_mail_and_no_count() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().into_owned();
        super::super::database::open_courly_database(path.clone()).expect("migrate");
        assert!(list_accounts(path.clone()).expect("accounts").is_empty());
        for folder in MailFolder::ALL {
            let threads = folder_threads(path.clone(), None, folder, 50).expect("threads");
            assert!(threads.is_empty(), "{folder:?}");
        }
        assert!(unread_counts(path).expect("counts").is_empty());
    }
}
