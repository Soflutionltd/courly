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
