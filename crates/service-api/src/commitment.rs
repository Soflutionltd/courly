//! Commitment wire types: `commitment.save` creates a commitment or
//! replaces its content, `commitment.delete` removes it. A commitment is
//! something the user owes or is waiting for, written down from a mail.

use serde::{Deserialize, Serialize};

/// `commitment.save` request body. The id is the caller's: saving the same
/// id again replaces the commitment, so a replayed request changes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentSaveParams {
    pub id: String,
    pub account_id: String,
    /// "owed" for what the user owes, "awaited" for what they wait for.
    pub direction: String,
    pub text: String,
    pub person_name: Option<String>,
    pub person_address: Option<String>,
    /// The day it is due, as `YYYY-MM-DD`.
    pub due_date: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub done: bool,
}

/// `commitment.delete` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentDeleteParams {
    pub id: String,
}

/// Ack of `commitment.save` and `commitment.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentAck;
