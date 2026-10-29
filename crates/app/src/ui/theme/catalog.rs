use iced::theme::palette::Seed;
use iced::{Color, Theme};
use serde::Deserialize;

use super::color::hex_to_color;

/// Default light palette, modeled on Microsoft Outlook (Fluent brand blue).
pub const OUTLOOK_LIGHT: Seed = Seed {
    background: Color::WHITE,
    text: Color::from_rgb8(0x24, 0x24, 0x24),
    primary: Color::from_rgb8(0x0F, 0x6C, 0xBD),
    success: Color::from_rgb8(0x10, 0x7C, 0x10),
    warning: Color::from_rgb8(0xBC, 0x4B, 0x09),
    danger: Color::from_rgb8(0xC5, 0x0F, 0x1F),
};

/// Default dark palette, modeled on Microsoft Outlook's dark mode.
pub const OUTLOOK_DARK: Seed = Seed {
    background: Color::from_rgb8(0x29, 0x29, 0x29),
    text: Color::from_rgb8(0xF5, 0xF5, 0xF5),
    primary: Color::from_rgb8(0x47, 0x9E, 0xF5),
    success: Color::from_rgb8(0x54, 0xB0, 0x54),
    warning: Color::from_rgb8(0xF7, 0x63, 0x0C),
    danger: Color::from_rgb8(0xDC, 0x62, 0x6D),
};

pub fn light_theme() -> Theme {
    Theme::custom(String::from("Light"), OUTLOOK_LIGHT)
}

pub fn dark_theme() -> Theme {
    Theme::custom(String::from("Dark"), OUTLOOK_DARK)
}

pub struct ThemeEntry {
    pub name: &'static str,
    pub palette: Seed,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry {
        name: "Light",
        palette: OUTLOOK_LIGHT,
    },
    ThemeEntry {
        name: "Dark",
        palette: OUTLOOK_DARK,
    },
    ThemeEntry {
        name: "Dracula",
        palette: Seed::DRACULA,
    },
    ThemeEntry {
        name: "Nord",
        palette: Seed::NORD,
    },
    ThemeEntry {
        name: "Solarized Light",
        palette: Seed::SOLARIZED_LIGHT,
    },
    ThemeEntry {
        name: "Solarized Dark",
        palette: Seed::SOLARIZED_DARK,
    },
    ThemeEntry {
        name: "Gruvbox Light",
        palette: Seed::GRUVBOX_LIGHT,
    },
    ThemeEntry {
        name: "Gruvbox Dark",
        palette: Seed::GRUVBOX_DARK,
    },
    ThemeEntry {
        name: "Catppuccin Latte",
        palette: Seed::CATPPUCCIN_LATTE,
    },
    ThemeEntry {
        name: "Catppuccin Frappé",
        palette: Seed::CATPPUCCIN_FRAPPE,
    },
    ThemeEntry {
        name: "Catppuccin Macchiato",
        palette: Seed::CATPPUCCIN_MACCHIATO,
    },
    ThemeEntry {
        name: "Catppuccin Mocha",
        palette: Seed::CATPPUCCIN_MOCHA,
    },
    ThemeEntry {
        name: "Tokyo Night",
        palette: Seed::TOKYO_NIGHT,
    },
    ThemeEntry {
        name: "Tokyo Night Storm",
        palette: Seed::TOKYO_NIGHT_STORM,
    },
    ThemeEntry {
        name: "Tokyo Night Light",
        palette: Seed::TOKYO_NIGHT_LIGHT,
    },
    ThemeEntry {
        name: "Kanagawa Wave",
        palette: Seed::KANAGAWA_WAVE,
    },
    ThemeEntry {
        name: "Kanagawa Lotus",
        palette: Seed::KANAGAWA_LOTUS,
    },
    ThemeEntry {
        name: "Moonfly",
        palette: Seed::MOONFLY,
    },
    ThemeEntry {
        name: "Nightfly",
        palette: Seed::NIGHTFLY,
    },
    ThemeEntry {
        name: "Oxocarbon",
        palette: Seed::OXOCARBON,
    },
    ThemeEntry {
        name: "Ferra",
        palette: Seed::FERRA,
    },
];

pub fn theme_by_index(index: usize) -> Theme {
    let entry = &THEMES[index.min(THEMES.len() - 1)];
    Theme::custom(entry.name.to_string(), entry.palette)
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    colors: ThemeColors,
}

#[derive(Debug, Deserialize)]
struct ThemeColors {
    background: String,
    text: String,
    primary: String,
    success: String,
    warning: String,
    danger: String,
}

pub fn from_toml(content: &str) -> Result<Theme, toml::de::Error> {
    let file: ThemeFile = toml::from_str(content)?;
    let palette = Seed {
        background: hex_to_color(&file.colors.background),
        text: hex_to_color(&file.colors.text),
        primary: hex_to_color(&file.colors.primary),
        success: hex_to_color(&file.colors.success),
        warning: hex_to_color(&file.colors.warning),
        danger: hex_to_color(&file.colors.danger),
    };
    Ok(Theme::custom(
        file.name.unwrap_or_else(|| "Custom".into()),
        palette,
    ))
}
