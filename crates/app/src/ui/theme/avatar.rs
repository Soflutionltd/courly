use iced::Color;

use super::color::hsl_to_color;

const AVATAR_HUES: &[f32] = &[
    260.0, // indigo
    160.0, // green
    25.0,  // red-orange
    45.0,  // amber
    290.0, // purple
    195.0, // cyan
    340.0, // pink
    130.0, // emerald
];

pub fn avatar_color(name: &str) -> Color {
    let hash: usize = name.bytes().map(|b| b as usize).sum();
    let hue = AVATAR_HUES[hash % AVATAR_HUES.len()];
    hsl_to_color(hue, 0.65, 0.55)
}

/// Up to two initials, Outlook style: first and last word of a display
/// name ("Julie Reibell" is "JR"). A bare address uses its local part, split
/// on dots, dashes and underscores ("jean.dupont@x.fr" is "JD").
pub fn initials(name: &str) -> String {
    let name = name.trim();
    let (source, separators): (&str, &[char]) = match name.split_once('@') {
        Some((local, _)) => (local, &['.', '_', '-', '+']),
        None => (name, &[]),
    };
    let firsts: Vec<char> = source
        .split(|c: char| c.is_whitespace() || separators.contains(&c))
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .collect();
    let picked = match firsts.as_slice() {
        [] => return "?".to_string(),
        [only] => vec![*only],
        [first, .., last] => vec![*first, *last],
    };
    picked.into_iter().flat_map(char::to_uppercase).collect()
}

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn display_names_use_first_and_last_word() {
        assert_eq!(initials("Julie Reibell"), "JR");
        assert_eq!(initials("Do Not Reply"), "DR");
        assert_eq!(initials("Uber"), "U");
        assert_eq!(initials("  \"élodie\" Martin "), "ÉM");
    }

    #[test]
    fn addresses_use_the_local_part() {
        assert_eq!(initials("jean.dupont@example.com"), "JD");
        assert_eq!(initials("antoine@pinelli.fr"), "A");
    }

    #[test]
    fn empty_names_fall_back() {
        assert_eq!(initials(""), "?");
        assert_eq!(initials("---"), "?");
    }
}
