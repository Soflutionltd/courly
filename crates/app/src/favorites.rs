//! People the reader starred as favourites, kept in the `favorite_contacts`
//! setting and listed in the folder column: a click shows the mail that
//! person sent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteContact {
    pub address: String,
    /// The name shown for the person; their address when none is known.
    pub name: String,
}

/// The favourites, in the order they were starred.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorites(Vec<FavoriteContact>);

impl Favorites {
    /// Read the saved setting. An unreadable value starts an empty list
    /// rather than failing the boot.
    pub fn parse(json: Option<&str>) -> Self {
        let Some(json) = json else {
            return Self::default();
        };
        serde_json::from_str(json).map(Self).unwrap_or_else(|e| {
            log::warn!("favorite_contacts: unreadable saved list, starting empty: {e}");
            Self::default()
        })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|e| {
            log::error!("favorite_contacts: could not serialise the list: {e}");
            "[]".to_owned()
        })
    }

    pub fn contacts(&self) -> &[FavoriteContact] {
        &self.0
    }

    /// The favourite with this address, whatever its letter case.
    pub fn get(&self, address: &str) -> Option<&FavoriteContact> {
        self.0
            .iter()
            .find(|contact| contact.address.eq_ignore_ascii_case(address))
    }

    /// Star the person, or unstar them when they already are a favourite.
    pub fn toggle(&mut self, address: &str, name: &str) {
        let before = self.0.len();
        self.0
            .retain(|contact| !contact.address.eq_ignore_ascii_case(address));
        if self.0.len() == before {
            self.0.push(FavoriteContact {
                address: address.to_owned(),
                name: name.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starring_twice_removes_the_person_whatever_the_letter_case() {
        let mut favorites = Favorites::default();
        favorites.toggle("ann@example.com", "Ann Example");
        assert!(favorites.get("Ann@Example.com").is_some());
        favorites.toggle("ANN@example.com", "Ann Example");
        assert!(favorites.contacts().is_empty());
    }

    #[test]
    fn the_list_survives_a_save_and_a_bad_value_starts_empty() {
        let mut favorites = Favorites::default();
        favorites.toggle("ann@example.com", "Ann Example");
        favorites.toggle("bob@example.com", "bob@example.com");
        assert_eq!(Favorites::parse(Some(&favorites.to_json())), favorites);
        assert_eq!(Favorites::parse(Some("not json")), Favorites::default());
        assert_eq!(Favorites::parse(None), Favorites::default());
    }
}
