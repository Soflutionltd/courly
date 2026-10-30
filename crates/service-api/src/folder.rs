//! Wire types for provider folder IPC (`folder.*`). Folder ids are local
//! storage ids; system folders (Inbox, Sent, Trash...) are refused.

use serde::{Deserialize, Serialize};

/// `folder.create` params. `parent_id: None` creates a top-level folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderCreateParams {
    pub account_id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

/// `folder.rename` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRenameParams {
    pub account_id: String,
    pub folder_id: String,
    pub name: String,
}

/// `folder.move` params. `parent_id: None` moves the folder to the top level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderMoveParams {
    pub account_id: String,
    pub folder_id: String,
    pub parent_id: Option<String>,
}

/// `folder.delete` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderDeleteParams {
    pub account_id: String,
    pub folder_id: String,
}

/// Ack of `folder.create` / `folder.rename`: the folder's storage id after
/// the write (a rename re-keys path-based IMAP folders).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderWriteAck {
    pub folder_id: String,
}

/// Ack of `folder.move` / `folder.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderDoneAck;
