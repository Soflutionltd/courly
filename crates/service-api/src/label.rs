//! Wire types for label_group IPC.

use serde::{Deserialize, Serialize};

/// `label_group.reorder` params. Each pair is `(label_groups.id, sort_order)`.
/// The Service writes all rows in one transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupReorderParams {
    pub orders: Vec<(i64, i64)>,
}

/// `label_group.reorder` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupReorderAck;

/// `label_group.save` params: create (`group_id: None`) or change one
/// user-visible label.
///
/// A new label with no `members` is created on the provider of every
/// account of `accounts`, an Outlook category or a Gmail label of that name. With
/// `members`, it gathers those provider labels instead and nothing is
/// created. On a change, the name is local (an Outlook category cannot be
/// renamed), the colour is also written to the provider labels of the
/// group, and `members: None` leaves the membership as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupSaveParams {
    pub group_id: Option<i64>,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
    /// `(account_id, label_id)` pairs.
    pub members: Option<Vec<(String, String)>>,
    /// Accounts a new label is created on. Unused on a change.
    pub accounts: Vec<String>,
}

/// `label_group.save` ack: the id of the created or changed group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupSaveAck {
    pub group_id: i64,
}

/// `label_group.delete` params. The provider labels of the group are
/// deleted on their providers too: left there, the next sync would bring
/// the group back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupDeleteParams {
    pub group_id: i64,
}

/// `label_group.delete` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupDeleteAck;

/// `label.save` params. `label_id: None` creates the label on the
/// provider; otherwise the existing label is renamed and/or recoloured.
/// `color` is `(bg_hex, fg_hex)`; `None` leaves the provider colour as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSaveParams {
    pub account_id: String,
    pub label_id: Option<String>,
    pub name: String,
    pub color: Option<(String, String)>,
}

/// `label.save` ack: the storage id of the created or updated label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSaveAck {
    pub label_id: String,
}

/// `label.delete` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDeleteParams {
    pub account_id: String,
    pub label_id: String,
}

/// `label.delete` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDeleteAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_group_reorder_params_round_trip_through_serde() {
        let original = LabelGroupReorderParams {
            orders: vec![(1, 0), (2, 1), (3, 2)],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: LabelGroupReorderParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn label_group_save_params_round_trip_through_serde() {
        let original = LabelGroupSaveParams {
            group_id: Some(4),
            name: "À traiter".to_owned(),
            color_bg: "#e7a1ad".to_owned(),
            color_fg: "#000000".to_owned(),
            members: Some(vec![("acc".to_owned(), "cat:À traiter".to_owned())]),
            accounts: vec!["acc".to_owned()],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: LabelGroupSaveParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn label_save_params_round_trip_through_serde() {
        let original = LabelSaveParams {
            account_id: "acc".to_owned(),
            label_id: Some("cat:À traiter".to_owned()),
            name: "À traiter".to_owned(),
            color: Some(("#e7a1ad".to_owned(), "#000000".to_owned())),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: LabelSaveParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
