//! Blocked sender wire types: `sender.block` adds an address to an
//! account's blocked list, `sender.unblock` removes it. Mail from a blocked
//! address goes to Spam; neither call moves mail that is already stored.

use serde::{Deserialize, Serialize};

/// `sender.block` / `sender.unblock` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderBlockParams {
    pub account_id: String,
    pub sender_address: String,
}

/// `sender.block` ack. `newly_blocked` is false when the address was
/// already on the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderBlockAck {
    pub newly_blocked: bool,
}

/// `sender.unblock` ack. `was_blocked` is false when the address was not on
/// the list; unblocking it is then a no-op, not an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderUnblockAck {
    pub was_blocked: bool,
}
