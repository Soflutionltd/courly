//! What the mailbox knows of one sender address: since when it writes,
//! whether its mail passes DMARC, and whether it is a contact. This is what
//! decides if a brand logo may stand beside its messages.

use crate::db::ReadConn;
use rusqlite::params;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SenderHistory {
    /// Date of the oldest message from the address, in milliseconds.
    pub first_message_ms: Option<i64>,
    /// Messages since the window start whose DMARC check passed.
    pub dmarc_passes: i64,
    /// Messages since the window start whose DMARC check failed.
    pub dmarc_failures: i64,
    pub is_contact: bool,
}

/// The history of `address`. DMARC verdicts are counted on messages dated
/// `auth_since_ms` or later. Runs on the `from_address` index.
pub fn sender_history_sync(
    conn: &ReadConn<'_>,
    address: &str,
    auth_since_ms: i64,
) -> Result<SenderHistory, String> {
    let normalized = address.trim().to_lowercase();
    let (first_message_ms, dmarc_passes, dmarc_failures) = conn
        .query_row(
            "SELECT MIN(date) AS first_date, \
                    COALESCE(SUM(CASE WHEN date >= ?3 AND auth_results LIKE '%dmarc=pass%' \
                                      THEN 1 ELSE 0 END), 0) AS passes, \
                    COALESCE(SUM(CASE WHEN date >= ?3 AND auth_results LIKE '%dmarc=fail%' \
                                      THEN 1 ELSE 0 END), 0) AS failures \
             FROM messages WHERE from_address IN (?1, ?2)",
            params![normalized, address.trim(), auth_since_ms],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>("first_date")?,
                    row.get::<_, i64>("passes")?,
                    row.get::<_, i64>("failures")?,
                ))
            },
        )
        .map_err(|e| format!("sender history: {e}"))?;
    let is_contact = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM contacts WHERE email = ?1) AS known",
            params![normalized],
            |row| row.get::<_, bool>("known"),
        )
        .map_err(|e| format!("sender history, contact: {e}"))?;
    Ok(SenderHistory {
        first_message_ms,
        dmarc_passes,
        dmarc_failures,
        is_contact,
    })
}
