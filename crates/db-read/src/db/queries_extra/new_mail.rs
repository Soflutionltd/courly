use crate::db::ReadConn;

/// An unread inbox message, as a desktop notification needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMailRow {
    pub account_id: String,
    pub message_id: String,
    pub thread_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// Inbox category of the thread, "Primary" when it has none.
    pub bundle: String,
}

/// Unread inbox messages dated at or after `since_ms`, newest first, from
/// every account or only `account_id`. Muted threads and messages sent from
/// the account's own address are left out.
pub fn unread_inbox_messages_since_sync(
    conn: &ReadConn<'_>,
    account_id: Option<&str>,
    since_ms: i64,
    limit: i64,
) -> Result<Vec<NewMailRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.account_id, m.id, m.thread_id, m.from_name, m.from_address,
                    m.subject, m.snippet, COALESCE(tb.bundle, 'Primary') AS bundle
             FROM messages m
             JOIN message_folders mf
               ON mf.account_id = m.account_id AND mf.message_id = m.id
              AND mf.folder_id = 'INBOX'
             JOIN threads t ON t.account_id = m.account_id AND t.id = m.thread_id
             JOIN accounts a ON a.id = m.account_id
             LEFT JOIN thread_bundles tb
               ON tb.account_id = m.account_id AND tb.thread_id = m.thread_id
             WHERE m.is_read = 0
               AND m.date >= ?1
               AND COALESCE(t.is_muted, 0) = 0
               AND lower(COALESCE(m.from_address, '')) <> lower(a.email)
               AND (?2 IS NULL OR m.account_id = ?2)
             ORDER BY m.date DESC
             LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map(rusqlite::params![since_ms, account_id, limit], |row| {
        Ok(NewMailRow {
            account_id: row.get("account_id")?,
            message_id: row.get("id")?,
            thread_id: row.get("thread_id")?,
            from_name: row.get("from_name")?,
            from_address: row.get("from_address")?,
            subject: row.get("subject")?,
            snippet: row.get("snippet")?,
            bundle: row.get("bundle")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
