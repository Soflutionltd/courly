//! Commitments as the app lists them. They are written by the service,
//! through the `db` crate.

use super::super::types::DbCommitment;
use crate::db::{ReadConn, query_as};

const COLUMNS: &str = "id, account_id, direction, text, person_name, person_address, \
     due_date, thread_id, message_id, done_at, created_at, updated_at";

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
