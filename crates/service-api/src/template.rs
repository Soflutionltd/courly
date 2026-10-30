//! Email template wire types: `template.save` creates or updates one
//! template, `template.delete` removes one. Templates live only in the
//! local database; no provider sees them.

use serde::{Deserialize, Serialize};

/// `template.save` request body. `id` set updates that template, unset
/// creates one. An empty or blank `subject` is stored as no subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSaveParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub body_html: String,
}

/// `template.save` ack: the id of the stored template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateSaveAck {
    pub id: String,
}

/// `template.delete` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDeleteParams {
    pub id: String,
}

/// `template.delete` ack. Deleting a missing template is not an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDeleteAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_params_omit_absent_id_and_subject() {
        let params = TemplateSaveParams {
            id: None,
            name: "Relance".to_string(),
            subject: None,
            body_html: "<p>Bonjour,</p>".to_string(),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(!object.contains_key("id"));
        assert!(!object.contains_key("subject"));
        let recovered: TemplateSaveParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }
}
