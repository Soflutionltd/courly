//! Addresses the reader no longer wants suggested, kept in the
//! `hidden_addresses` setting.
//!
//! The people suggested while typing come from the address book and from
//! the addresses seen in mail. That second list is rebuilt from the mail
//! itself, so removing a mistyped address from it would not last: the next
//! sync brings it back. Hiding it does last, and can be undone.

/// The hidden addresses, lowercase, in the order they were hidden.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenAddresses(Vec<String>);

impl HiddenAddresses {
    /// Read the saved setting. An unreadable value starts an empty list
    /// rather than failing the boot.
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        serde_json::from_str(json).map(Self).unwrap_or_else(|e| {
            log::warn!("hidden_addresses: unreadable saved list, starting empty: {e}");
            Self::default()
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|e| {
            log::error!("hidden_addresses: could not serialise the list: {e}");
            "[]".to_owned()
        })
    }

    pub fn addresses(&self) -> &[String] {
        &self.0
    }

    /// Whether this address is hidden, whatever its letter case.
    pub fn hides(&self, address: &str) -> bool {
        let address = address.trim();
        self.0
            .iter()
            .any(|hidden| hidden.eq_ignore_ascii_case(address))
    }

    /// Hide the address. `false` when it is empty or already hidden.
    pub fn hide(&mut self, address: &str) -> bool {
        let address = address.trim().to_lowercase();
        if address.is_empty() || self.hides(&address) {
            return false;
        }
        self.0.push(address);
        true
    }

    /// Suggest the address again. `false` when it was not hidden.
    pub fn restore(&mut self, address: &str) -> bool {
        let before = self.0.len();
        let address = address.trim();
        self.0
            .retain(|hidden| !hidden.eq_ignore_ascii_case(address));
        self.0.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_address_is_hidden_whatever_its_letter_case() {
        let mut hidden = HiddenAddresses::default();
        assert!(hidden.hide(" Ann@Example.com "));
        assert!(hidden.hides("ann@example.COM"));
        assert!(!hidden.hide("ANN@example.com"), "already hidden");
        assert!(!hidden.hide("  "), "nothing to hide");
        assert_eq!(hidden.addresses(), ["ann@example.com"]);
    }

    #[test]
    fn a_restored_address_is_suggested_again() {
        let mut hidden = HiddenAddresses::default();
        hidden.hide("ann@example.com");
        assert!(hidden.restore("Ann@example.com"));
        assert!(!hidden.hides("ann@example.com"));
        assert!(!hidden.restore("ann@example.com"), "was not hidden");
    }

    #[test]
    fn the_list_survives_a_save_and_a_bad_value_starts_empty() {
        let mut hidden = HiddenAddresses::default();
        hidden.hide("ann@example.com");
        hidden.hide("bob@example.com");
        assert_eq!(HiddenAddresses::parse(Some(&hidden.to_json())), hidden);
        assert_eq!(
            HiddenAddresses::parse(Some("not json")),
            HiddenAddresses::default()
        );
        assert_eq!(HiddenAddresses::parse(None), HiddenAddresses::default());
    }
}
