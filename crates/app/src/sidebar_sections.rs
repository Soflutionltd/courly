//! Which optional parts of the folder column are shown, kept in the
//! `sidebar_hidden` setting.
//!
//! The setting lists what is hidden, so a part added later shows by default
//! for everyone who never touched the setting.

/// A part of the folder column the reader can take out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    FavoriteContacts,
    /// The row listing starred mail, among the folders.
    StarredMail,
    SmartFolders,
    Labels,
}

impl SidebarSection {
    pub const ALL: [Self; 4] = [
        Self::FavoriteContacts,
        Self::StarredMail,
        Self::SmartFolders,
        Self::Labels,
    ];

    /// The word standing for the part in the setting.
    fn key(self) -> &'static str {
        match self {
            Self::FavoriteContacts => "contacts",
            Self::StarredMail => "starred",
            Self::SmartFolders => "smart",
            Self::Labels => "labels",
        }
    }
}

/// The parts taken out of the folder column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenSections(Vec<SidebarSection>);

impl Default for HiddenSections {
    /// Before the reader chooses: favourite people are listed at the top of
    /// the column, so the row of starred mail, the other "favourites", is
    /// the one left out.
    fn default() -> Self {
        Self(vec![SidebarSection::StarredMail])
    }
}

impl HiddenSections {
    /// Read the setting: nothing stored is the default, an empty value is
    /// "everything shown". Unknown words are ignored.
    pub fn parse(stored: Option<&str>) -> Self {
        let Some(stored) = stored else {
            return Self::default();
        };
        Self(
            SidebarSection::ALL
                .into_iter()
                .filter(|section| stored.split(',').any(|key| key.trim() == section.key()))
                .collect(),
        )
    }

    pub fn to_setting(&self) -> String {
        self.0
            .iter()
            .map(|section| section.key())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn shows(&self, section: SidebarSection) -> bool {
        !self.0.contains(&section)
    }

    pub fn set_shown(&mut self, section: SidebarSection, shown: bool) {
        self.0.retain(|hidden| *hidden != section);
        if !shown {
            self.0.push(section);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_hides_starred_mail_only() {
        let hidden = HiddenSections::parse(None);
        assert!(!hidden.shows(SidebarSection::StarredMail));
        assert!(hidden.shows(SidebarSection::FavoriteContacts));
        assert!(hidden.shows(SidebarSection::SmartFolders));
        assert!(hidden.shows(SidebarSection::Labels));
    }

    #[test]
    fn an_empty_setting_shows_everything() {
        let hidden = HiddenSections::parse(Some(""));
        assert!(
            SidebarSection::ALL
                .into_iter()
                .all(|section| hidden.shows(section))
        );
    }

    #[test]
    fn a_choice_survives_being_stored() {
        let mut hidden = HiddenSections::default();
        hidden.set_shown(SidebarSection::StarredMail, true);
        hidden.set_shown(SidebarSection::SmartFolders, false);
        hidden.set_shown(SidebarSection::Labels, false);
        hidden.set_shown(SidebarSection::Labels, false);
        let stored = hidden.to_setting();
        assert_eq!(stored, "smart,labels");
        assert_eq!(HiddenSections::parse(Some(&stored)), hidden);
        assert_eq!(
            HiddenSections::parse(Some("smart, nonsense")),
            HiddenSections(vec![SidebarSection::SmartFolders])
        );
    }
}
