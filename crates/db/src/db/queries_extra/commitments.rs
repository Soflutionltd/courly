//! Commitments: what the user owes someone, and what someone owes the user.
//!
//! A commitment is written down from a mail and outlives it: it keeps the
//! sentence and the person itself, so deleting or filing the mail away
//! takes nothing from it. The mail is only where it can be read again.

use super::super::WriteConn;
use super::super::types::DbCommitment;
use crate::db::{ReadConn, query_as};
use rusqlite::params;

const COLUMNS: &str = "id, account_id, direction, text, person_name, person_address, \
     due_date, thread_id, message_id, done_at, created_at, updated_at";

/// `commitments.direction` of something the user owes.
pub const COMMITMENT_OWED: &str = "owed";

/// `commitments.direction` of something the user is waiting for.
pub const COMMITMENT_AWAITED: &str = "awaited";

/// The content of a commitment, as the user wrote or changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitmentSave {
    pub id: String,
    pub account_id: String,
    pub direction: String,
    pub text: String,
    pub person_name: Option<String>,
    pub person_address: Option<String>,
    /// The day it is due, as `YYYY-MM-DD`, when it has one.
    pub due_date: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub done: bool,
}

fn checked(save: &CommitmentSave) -> Result<(), String> {
    if save.direction != COMMITMENT_OWED && save.direction != COMMITMENT_AWAITED {
        return Err(format!("unknown commitment direction {:?}", save.direction));
    }
    if save.text.trim().is_empty() {
        return Err("a commitment needs a text".to_owned());
    }
    if let Some(day) = &save.due_date
        && !is_iso_day(day)
    {
        return Err(format!("due date {day:?} is not a YYYY-MM-DD day"));
    }
    Ok(())
}

fn is_iso_day(day: &str) -> bool {
    let bytes = day.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

/// Create the commitment `save.id`, or replace its content. `done_at` keeps
/// the moment it was first closed, and is cleared when it is reopened.
pub fn save_commitment(conn: &WriteConn<'_>, save: &CommitmentSave) -> Result<(), String> {
    checked(save)?;
    conn.execute(
        "INSERT INTO commitments \
           (id, account_id, direction, text, person_name, person_address, due_date, \
            thread_id, message_id, done_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CASE WHEN ?10 THEN unixepoch() END) \
         ON CONFLICT(id) DO UPDATE SET \
           direction = excluded.direction, text = excluded.text, \
           person_name = excluded.person_name, person_address = excluded.person_address, \
           due_date = excluded.due_date, \
           done_at = CASE WHEN ?10 THEN COALESCE(commitments.done_at, unixepoch()) END, \
           updated_at = unixepoch()",
        params![
            save.id,
            save.account_id,
            save.direction,
            save.text.trim(),
            save.person_name,
            save.person_address,
            save.due_date,
            save.thread_id,
            save.message_id,
            save.done,
        ],
    )
    .map_err(|e| format!("save commitment: {e}"))?;
    Ok(())
}

/// Delete the commitment `id`. Deleting one that is gone is not an error.
pub fn delete_commitment(conn: &WriteConn<'_>, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM commitments WHERE id = ?1", params![id])
        .map_err(|e| format!("delete commitment: {e}"))?;
    Ok(())
}

/// Every open commitment, and those closed since `closed_since` (unix
/// seconds), soonest due first; one without a due date comes after those
/// that have one.
pub fn list_commitments(
    conn: &ReadConn<'_>,
    closed_since: i64,
) -> Result<Vec<DbCommitment>, String> {
    query_as::<DbCommitment>(
        conn,
        &format!(
            "SELECT {COLUMNS} FROM commitments \
             WHERE done_at IS NULL OR done_at >= ?1 \
             ORDER BY done_at IS NOT NULL, due_date IS NULL, due_date, created_at"
        ),
        &[&closed_since],
    )
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

    fn save(id: &str, direction: &str, due: Option<&str>) -> CommitmentSave {
        CommitmentSave {
            id: id.to_owned(),
            account_id: "acc".to_owned(),
            direction: direction.to_owned(),
            text: format!("  {id} text "),
            person_name: Some("Ada".to_owned()),
            person_address: Some("ada@example.com".to_owned()),
            due_date: due.map(str::to_owned),
            thread_id: Some("thread".to_owned()),
            message_id: Some("message".to_owned()),
            done: false,
        }
    }

    fn ids(conn: &WriteConn<'_>, closed_since: i64) -> Vec<String> {
        list_commitments(&conn.as_read(), closed_since)
            .expect("list")
            .into_iter()
            .map(|commitment| commitment.id)
            .collect()
    }

    #[test]
    fn open_commitments_come_soonest_due_first_and_undated_last() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        save_commitment(&conn, &save("undated", COMMITMENT_OWED, None)).expect("save");
        save_commitment(&conn, &save("late", COMMITMENT_AWAITED, Some("2026-12-20")))
            .expect("save");
        save_commitment(&conn, &save("soon", COMMITMENT_OWED, Some("2026-12-01"))).expect("save");

        assert_eq!(ids(&conn, i64::MAX), vec!["soon", "late", "undated"]);
        let listed = list_commitments(&conn.as_read(), i64::MAX).expect("list");
        assert_eq!(listed[0].text, "soon text");
        assert_eq!(listed[0].done_at, None);
    }

    #[test]
    fn closing_keeps_the_commitment_listed_until_the_cutoff_and_reopening_clears_it() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        let mut commitment = save("a", COMMITMENT_OWED, Some("2026-12-01"));
        save_commitment(&conn, &commitment).expect("create");

        commitment.done = true;
        save_commitment(&conn, &commitment).expect("close");
        assert!(ids(&conn, i64::MAX).is_empty(), "closed before the cutoff");
        let closed = list_commitments(&conn.as_read(), 0).expect("list");
        assert!(closed[0].done_at.is_some());

        commitment.done = false;
        save_commitment(&conn, &commitment).expect("reopen");
        assert_eq!(ids(&conn, i64::MAX), vec!["a"]);
    }

    #[test]
    fn a_commitment_survives_its_mail_and_goes_with_its_account() {
        let raw = open_with_account();
        raw.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragma");
        let conn = WriteConn::from_raw(&raw);
        // The thread and message named do not exist: nothing ties the row
        // to them.
        save_commitment(&conn, &save("a", COMMITMENT_AWAITED, None)).expect("save");
        assert_eq!(ids(&conn, i64::MAX), vec!["a"]);

        raw.execute("DELETE FROM accounts WHERE id = 'acc'", [])
            .expect("delete account");
        assert!(ids(&conn, i64::MAX).is_empty());
    }

    #[test]
    fn a_malformed_commitment_is_refused() {
        let raw = open_with_account();
        let conn = WriteConn::from_raw(&raw);
        assert!(save_commitment(&conn, &save("a", "promised", None)).is_err());
        assert!(save_commitment(&conn, &save("a", COMMITMENT_OWED, Some("1 dec"))).is_err());
        let mut blank = save("a", COMMITMENT_OWED, None);
        blank.text = "   ".to_owned();
        assert!(save_commitment(&conn, &blank).is_err());

        delete_commitment(&conn, "never-existed").expect("deleting nothing is fine");
    }
}
