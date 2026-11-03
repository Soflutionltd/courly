//! The fields the contact editor shows, and how an edit of one of them
//! lands in an address book's card, which may hold more values than
//! Courly shows (other numbers, other addresses, labels).

/// The fields of a contact the editor shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ContactFields {
    pub display_name: Option<String>,
    pub email: String,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
}

impl ContactFields {
    /// The addresses, primary first.
    pub(crate) fn emails(&self) -> Vec<String> {
        std::iter::once(self.email.clone())
            .chain(self.email2.clone())
            .collect()
    }

    /// Given and family name for address books that store them apart: the
    /// first word is the given name, the rest the family name.
    pub(crate) fn split_name(&self) -> (String, String) {
        let name = self.display_name.as_deref().unwrap_or("").trim();
        match name.split_once(char::is_whitespace) {
            Some((given, family)) => (given.to_owned(), family.trim().to_owned()),
            None => (name.to_owned(), String::new()),
        }
    }
}

/// How one value of a card changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueEdit<'a> {
    Keep,
    Replace { old: &'a str, new: &'a str },
    Add(&'a str),
    Remove(&'a str),
}

impl<'a> ValueEdit<'a> {
    pub(crate) fn between(old: Option<&'a str>, new: Option<&'a str>) -> Self {
        match (old, new) {
            (Some(old), Some(new)) if old == new => Self::Keep,
            (Some(old), Some(new)) => Self::Replace { old, new },
            (None, Some(new)) => Self::Add(new),
            (Some(old), None) => Self::Remove(old),
            (None, None) => Self::Keep,
        }
    }

    /// Applies the edit to `items`, whose value `value_of` reads and
    /// `set_value` writes; `make` builds an added one. A replaced value
    /// keeps its position and label; a value the card lacks is added.
    pub(crate) fn apply<T>(
        self,
        items: &mut Vec<T>,
        value_of: impl Fn(&T) -> String,
        set_value: impl Fn(&mut T, &str),
        make: impl Fn(&str) -> T,
        same: impl Fn(&str, &str) -> bool,
    ) {
        match self {
            Self::Keep => {}
            Self::Replace { old, new } => {
                match items.iter_mut().find(|item| same(&value_of(item), old)) {
                    Some(item) => set_value(item, new),
                    None => Self::Add(new).apply(items, value_of, set_value, make, same),
                }
            }
            Self::Add(new) => {
                if !items.iter().any(|item| same(&value_of(item), new)) {
                    items.push(make(new));
                }
            }
            Self::Remove(old) => items.retain(|item| !same(&value_of(item), old)),
        }
    }
}

pub(crate) fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Phone numbers match whatever their spacing or punctuation.
pub(crate) fn same_phone(a: &str, b: &str) -> bool {
    let digits = |value: &str| -> String {
        value
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect()
    };
    digits(a) == digits(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(values: &[&str], change: ValueEdit<'_>) -> Vec<String> {
        let mut items: Vec<String> = values.iter().map(|v| (*v).to_owned()).collect();
        change.apply(
            &mut items,
            String::clone,
            |item, value| value.clone_into(item),
            str::to_owned,
            same_address,
        );
        items
    }

    #[test]
    fn a_replaced_value_keeps_its_place_and_the_others_stay() {
        assert_eq!(
            edit(
                &["home@x.fr", "Work@X.fr", "old@x.fr"],
                ValueEdit::between(Some("work@x.fr"), Some("new@x.fr"))
            ),
            ["home@x.fr", "new@x.fr", "old@x.fr"]
        );
    }

    #[test]
    fn values_are_added_once_and_removed() {
        assert_eq!(
            edit(&["a@x.fr"], ValueEdit::between(None, Some("A@x.fr"))),
            ["a@x.fr"]
        );
        assert_eq!(
            edit(
                &["a@x.fr"],
                ValueEdit::between(Some("gone@x.fr"), Some("b@x.fr"))
            ),
            ["a@x.fr", "b@x.fr"]
        );
        assert_eq!(
            edit(
                &["a@x.fr", "b@x.fr"],
                ValueEdit::between(Some("b@x.fr"), None)
            ),
            ["a@x.fr"]
        );
    }

    #[test]
    fn phones_match_whatever_their_spacing() {
        assert!(same_phone("+33 6 12 34 56 78", "+33612345678"));
        assert!(!same_phone("06 12 34 56 78", "06 12 34 56 79"));
    }

    #[test]
    fn the_first_word_is_the_given_name() {
        let fields = ContactFields {
            display_name: Some("Jean Pierre Dupont".to_owned()),
            ..ContactFields::default()
        };
        assert_eq!(
            fields.split_name(),
            ("Jean".to_owned(), "Pierre Dupont".to_owned())
        );
    }
}
