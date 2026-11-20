//! The Exchange outbox, recognised by its display name.
//!
//! Graph syncs the outbox as a plain folder: no well-known role reaches the
//! local database, and its id is the provider's own. Its name, in the
//! language of the mailbox, is the one thing that tells it apart.

/// Id of the view that gathers the outbox of every account.
pub const OUTBOX_VIEW_ID: &str = "OUTBOX";

/// Display names of the Exchange outbox, lowercase.
const OUTBOX_NAMES: &[&str] = &[
    "outbox",
    "boîte d'envoi",
    "postausgang",
    "bandeja de salida",
    "posta in uscita",
    "caixa de saída",
    "postvak uit",
    "utkorgen",
    "udbakke",
    "utboks",
    "lähtevät",
    "skrzynka nadawcza",
    "pošta k odeslání",
    "giden kutusu",
    "исходящие",
    "送信トレイ",
    "发件箱",
    "寄件匣",
    "보낼 편지함",
];

/// Whether a folder with this display name is an outbox.
pub fn is_outbox_name(name: &str) -> bool {
    let name = name.trim().to_lowercase().replace('\u{2019}', "'");
    OUTBOX_NAMES.contains(&name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_name_ignores_case_and_curly_apostrophe() {
        assert!(is_outbox_name("Boîte d\u{2019}envoi"));
        assert!(is_outbox_name(" OUTBOX "));
        assert!(is_outbox_name("Исходящие"));
        assert!(!is_outbox_name("Hotel"));
    }
}
