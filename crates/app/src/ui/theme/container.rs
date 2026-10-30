use iced::widget::container;
use iced::{Color, Theme, border};

use super::{EMAIL_BODY_BG_PREF, ON_AVATAR};
use crate::ui::layout::{CHAT_BUBBLE_RADIUS, RADIO_CIRCLE_SIZE, RADIUS_LG, RADIUS_MD, RADIUS_SM};
use crate::ui::settings::types::EmailBodyBackground;

/// All custom container styles used in the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContainerClass {
    /// Darkest background (thread list).
    Base,
    /// Panel divider (normal).
    Divider,
    /// Panel divider (hovered/dragging).
    DividerHover,
    /// Content area (reading pane, settings content).
    Content,
    /// Sidebar background.
    Sidebar,
    /// Surface with subtle border (no radius).
    Surface,
    /// Elevated card with border and rounded corners.
    Elevated,
    /// Pill-shaped badge background.
    Badge,
    /// Message card with rounded border.
    MessageCard,
    /// Email body inset - always white for rendering fidelity.
    EmailBody,
    /// Attachment card beside an email body. Its text follows the theme,
    /// so it only takes the white body sheet in a light theme.
    AttachmentCard,
    /// Action bar (no radius).
    ActionBar,
    /// Floating tooltip/popover on primary background.
    Floating,
    /// Settings section card with shadow.
    SettingsSection,
    /// Select dropdown menu with shadow.
    SelectMenu,
    /// Selected theme preview ring.
    ThemeSelectedRing,
    /// Selected color-swatch focus ring (circular).
    ColorSwatchSelectedRing,
    /// Shortcut key badge.
    KeyBadge,
    /// Active drag-reorder row highlight.
    DraggingRow,
    /// Drop target outline while a drag-reorder hovers a row.
    DropTarget,
    /// Status bar background.
    StatusBar,
    /// Palette card (elevated container with shadow).
    PaletteCard,
    /// Palette selected result row.
    PaletteSelectedRow,
    /// Semi-transparent dark overlay behind modals.
    ModalBackdrop,
    /// Calendar day cell with subtle border.
    CalendarCell,
    /// Today's calendar cell (accent border/tint).
    CalendarCellToday,
    /// Non-current-month calendar cell (muted background).
    CalendarCellMuted,
    /// Mini-month selected date highlight.
    MiniMonthSelected,
    /// Time grid hour label cell (left column).
    TimeGridHourLabel,
    /// Time grid cell border (very subtle).
    TimeGridCell,
    /// Today column header highlight in time grid.
    TimeGridTodayHeader,
    /// Current-time indicator line (red/accent).
    TimeGridNowLine,
    /// Floating chord indicator badge (bottom-right).
    ChordIndicator,
    /// Chat bubble - sent by user (accent background).
    ChatBubbleSent,
    /// Chat bubble - received from contact (surface background).
    ChatBubbleReceived,
    /// Radio circle outer ring - selected state (primary color).
    RadioCircleSelected,
    /// Radio circle outer ring - unselected state (muted border).
    RadioCircleUnselected,
    /// Radio circle inner filled disk (rendered only when selected).
    RadioCircleInner,
    /// Modal `Modal` surface card. Window-like opaque background, generous
    /// rounding, and a soft drop shadow so the dialog reads as a discrete
    /// surface above the dimmed backdrop. Used by the `alert_dialog` /
    /// `form_dialog` primitives in `ui/dialog.rs`.
    DialogCard,
    /// Brand-colored strip across the top of the main window (search).
    TitleBar,
    /// Title-bar search box holding the scope pill and the search input.
    TitleSearchField,
    /// Window background the panes float on.
    Canvas,
    /// A main-window pane (folders, message list, reading pane) drawn as
    /// a rounded card on the canvas.
    Pane,
}

impl ContainerClass {
    pub fn style(self) -> fn(&Theme) -> container::Style {
        match self {
            Self::Base => style_base_container,
            Self::Divider => style_divider_container,
            Self::DividerHover => style_divider_hover_container,
            Self::Content => style_content_container,
            Self::Sidebar => style_sidebar_container,
            Self::Surface => style_surface_container,
            Self::Elevated => style_elevated_container,
            Self::Badge => style_badge_container,
            Self::MessageCard => style_message_card_container,
            Self::EmailBody => style_email_body_container,
            Self::AttachmentCard => style_attachment_card_container,
            Self::ActionBar => style_action_bar_container,
            Self::Floating => style_floating_container,
            Self::SettingsSection => style_settings_section_container,
            Self::SelectMenu => style_select_menu_container,
            Self::ThemeSelectedRing => style_theme_selected_ring,
            Self::ColorSwatchSelectedRing => style_color_swatch_selected_ring,
            Self::KeyBadge => style_key_badge_container,
            Self::DraggingRow => style_dragging_row_container,
            Self::DropTarget => style_drop_target_container,
            Self::StatusBar => style_status_bar_container,
            Self::PaletteCard => style_palette_card_container,
            Self::PaletteSelectedRow => style_palette_selected_row_container,
            Self::ModalBackdrop => style_modal_backdrop_container,
            Self::CalendarCell => style_calendar_cell_container,
            Self::CalendarCellToday => style_calendar_cell_today_container,
            Self::CalendarCellMuted => style_calendar_cell_muted_container,
            Self::MiniMonthSelected => style_mini_month_selected_container,
            Self::TimeGridHourLabel => style_time_grid_hour_label_container,
            Self::TimeGridCell => style_time_grid_cell_container,
            Self::TimeGridTodayHeader => style_time_grid_today_header_container,
            Self::TimeGridNowLine => style_time_grid_now_line_container,
            Self::ChordIndicator => style_chord_indicator_container,
            Self::ChatBubbleSent => style_chat_bubble_sent,
            Self::ChatBubbleReceived => style_chat_bubble_received,
            Self::DialogCard => style_dialog_card_container,
            Self::RadioCircleSelected => style_radio_circle_selected,
            Self::RadioCircleUnselected => style_radio_circle_unselected,
            Self::RadioCircleInner => style_radio_circle_inner,
            Self::TitleBar => style_title_bar_container,
            Self::TitleSearchField => style_title_search_field_container,
            Self::Canvas => style_canvas_container,
            Self::Pane => style_pane_container,
        }
    }
}

fn style_title_bar_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    let background = if p.is_dark {
        p.background.base.color
    } else {
        p.primary.base.color
    };
    container::Style {
        background: Some(background.into()),
        ..Default::default()
    }
}

/// Same surface as the title search input, so the pill and the input read
/// as one box.
fn style_title_search_field_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    let background = if p.is_dark {
        p.background.weak.color
    } else {
        Color::WHITE
    };
    container::Style {
        background: Some(background.into()),
        border: border::rounded(RADIUS_SM),
        ..Default::default()
    }
}

/// The canvas sits one step darker than the panes in both modes.
/// `background.weak` deviates toward the text color, which is darker in
/// light mode but lighter in dark mode, so dark mode mixes toward black.
fn style_canvas_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    let background = if p.is_dark {
        mix(p.background.base.color, Color::BLACK, 0.35)
    } else {
        p.background.weak.color
    };
    container::Style {
        background: Some(background.into()),
        ..Default::default()
    }
}

fn style_pane_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.12),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

fn style_base_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.base.color.into()),
        ..Default::default()
    }
}

fn style_divider_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.strong.color.into()),
        ..Default::default()
    }
}

fn style_divider_hover_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.strongest.color.into()),
        ..Default::default()
    }
}

fn style_content_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weakest.color.into()),
        ..Default::default()
    }
}

fn style_sidebar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weaker.color.into()),
        ..Default::default()
    }
}

fn style_surface_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_elevated_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

fn style_badge_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weak.color.into()),
        border: iced::Border {
            radius: RADIUS_LG.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn theme_is_light(theme: &Theme) -> bool {
    let bg = theme.palette().background.base.color;
    0.299 * bg.r + 0.587 * bg.g + 0.114 * bg.b > 0.5
}

fn email_body_is_white(theme: &Theme) -> bool {
    match EMAIL_BODY_BG_PREF.get() {
        EmailBodyBackground::AlwaysWhite => true,
        EmailBodyBackground::MatchTheme => false,
        EmailBodyBackground::Auto => theme_is_light(theme),
    }
}

/// In a light theme with white email bodies the whole card is white, so
/// the email reads on one sheet like Outlook instead of a white box framed
/// by the theme's tinted surface.
fn message_card_is_white(theme: &Theme) -> bool {
    email_body_is_white(theme) && theme_is_light(theme)
}

fn style_message_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    let background = if message_card_is_white(theme) {
        Color::WHITE
    } else {
        p.background.weaker.color
    };
    container::Style {
        background: Some(background.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn style_email_body_container(theme: &Theme) -> container::Style {
    let (bg, border_color) = if message_card_is_white(theme) {
        (Color::WHITE, Color::TRANSPARENT)
    } else if email_body_is_white(theme) {
        (Color::WHITE, Color::BLACK.scale_alpha(0.08))
    } else {
        (
            theme.palette().background.base.color,
            theme.palette().background.strongest.color.scale_alpha(0.15),
        )
    };
    container::Style {
        background: Some(bg.into()),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

fn style_attachment_card_container(theme: &Theme) -> container::Style {
    if theme_is_light(theme) {
        return style_email_body_container(theme);
    }
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

fn style_action_bar_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_floating_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.85).into()),
        border: iced::Border {
            color: p.primary.strong.color,
            width: 2.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_settings_section_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.15),
            offset: iced::Vector::ZERO,
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_select_menu_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_theme_selected_ring(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        border: iced::Border {
            color: p.primary.base.color,
            width: 2.0,
            radius: (RADIUS_MD + 4.0).into(),
        },
        ..Default::default()
    }
}

/// Circular focus ring for the selected color-palette swatch. The radius
/// is intentionally large enough to fully round any swatch container we
/// pair this with, since the swatch itself is rendered as a circle.
fn style_color_swatch_selected_ring(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        border: iced::Border {
            color: p.primary.base.color,
            width: 2.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

fn style_key_badge_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn style_dragging_row_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.1).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.3),
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn style_drop_target_container(theme: &Theme) -> container::Style {
    container::Style {
        border: iced::Border {
            color: theme.palette().primary.base.color,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn style_status_bar_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_palette_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            radius: RADIUS_LG.into(),
            width: 1.0,
            color: p.background.strong.color,
        },
        shadow: iced::Shadow {
            color: Color {
                a: 0.3,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
        ..Default::default()
    }
}

fn style_palette_selected_row_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.weak.color.into()),
        border: border::rounded(RADIUS_SM),
        ..Default::default()
    }
}

fn style_dialog_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 24.0,
        },
        ..Default::default()
    }
}

fn style_modal_backdrop_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.5,
            }
            .into(),
        ),
        ..Default::default()
    }
}

fn style_calendar_cell_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_calendar_cell_today_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.06).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.4),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_calendar_cell_muted_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_mini_month_selected_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.15).into()),
        border: iced::Border {
            radius: RADIUS_SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn style_time_grid_hour_label_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_cell_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_today_header_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.08).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.3),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_now_line_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.danger.base.color.into()),
        ..Default::default()
    }
}

fn style_chord_indicator_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: border::rounded(RADIUS_SM)
            .color(p.background.strongest.color.scale_alpha(0.3))
            .width(1.0),
        shadow: iced::Shadow {
            color: Color {
                a: 0.15,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 4.0,
        },
        ..Default::default()
    }
}

fn style_chat_bubble_sent(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.weak.color.into()),
        border: border::rounded(CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

fn style_chat_bubble_received(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: border::rounded(CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

fn style_radio_circle_selected(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().primary.base.color.into()),
        border: border::rounded(RADIO_CIRCLE_SIZE / 2.0),
        ..Default::default()
    }
}

fn style_radio_circle_unselected(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.5),
            width: 1.5,
            radius: (RADIO_CIRCLE_SIZE / 2.0).into(),
        },
        ..Default::default()
    }
}

fn style_radio_circle_inner(_theme: &Theme) -> container::Style {
    let inner_size = RADIO_CIRCLE_SIZE * 0.3;
    container::Style {
        background: Some(ON_AVATAR.into()),
        border: border::rounded(inner_size / 2.0),
        ..Default::default()
    }
}
