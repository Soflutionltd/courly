#![allow(dead_code)]

use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length, Theme};
use rust_i18n::t;

use cmdk::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::Message;
use crate::icon;
use crate::ui::layout::{
    ICON_LG, ICON_MD, ICON_RAIL, ICON_XL, PAD_BUTTON, PAD_DROPDOWN, PAD_ICON_BTN, RAIL_BUTTON_SIZE,
    SPACE_XS, SPACE_XXS, TEXT_LG, TEXT_SM,
};
use crate::ui::theme;

/// Build a toolbar button for a registered command.
///
/// Pulls label (including toggle resolution like Star/Unstar),
/// availability, and keybinding hint from the registry and
/// binding table. Unavailable commands render as disabled buttons
/// with muted text. Keybinding hints appear as tooltips.
pub fn command_button<'a>(
    id: CommandId,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };

    let label_text = text(label).size(TEXT_SM).style(label_style);
    let mut btn = button(container(label_text).align_y(Alignment::Center))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    let _ = keybinding;
    btn.into()
}

/// Build a toolbar button for a registered command, with an icon.
///
/// Same as [`command_button`] but prepends an icon glyph before the label.
pub fn command_icon_button<'a>(
    id: CommandId,
    ico: iced::widget::Text<'a>,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };
    let icon_style: fn(&Theme) -> text::Style = label_style;

    let content = row![
        container(ico.size(ICON_MD).style(icon_style)).align_y(Alignment::Center),
        container(text(label).size(TEXT_SM).style(label_style)).align_y(Alignment::Center),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let mut btn = button(content)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    let _ = keybinding;
    btn.into()
}

pub fn action_icon_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary)).align_y(Alignment::Center),
            container(text(label).size(TEXT_SM).style(text::secondary)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Action.style())
    .into()
}

pub fn compose_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
    button(
        container(
            row![
                container(icon::pencil().size(ICON_LG).color(theme::ON_AVATAR))
                    .align_y(Alignment::Center),
                container(
                    text(t!("main.sidebar.compose"))
                        .size(TEXT_LG)
                        .color(theme::ON_AVATAR)
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .width(Length::Fill)
    .into()
}

/// Filled brand-colored button with an icon and a label, sized to content
/// (the ribbon's "New mail").
pub fn primary_icon_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
    button(
        row![
            container(ico.size(ICON_LG).color(theme::ON_AVATAR)).align_y(Alignment::Center),
            container(text(label).size(TEXT_LG).color(theme::ON_AVATAR)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .into()
}

/// Square icon button for the vertical app rail (Mail, Calendar, Settings).
pub fn rail_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    let icon_style: fn(&Theme) -> text::Style = if active {
        theme::TextClass::Accent.style()
    } else {
        text::secondary
    };
    button(
        container(ico.size(ICON_RAIL).style(icon_style))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(on_press)
    .padding(0)
    .style(theme::ButtonClass::Nav { active }.style())
    .width(RAIL_BUTTON_SIZE)
    .height(RAIL_BUTTON_SIZE)
    .into()
}

/// Thin vertical rule separating groups of ribbon buttons.
pub fn ribbon_separator<'a, M: 'a>() -> Element<'a, M> {
    container(iced::widget::Space::new())
        .width(1)
        .height(ICON_RAIL)
        .style(theme::ContainerClass::Divider.style())
        .into()
}

/// Icon-only "+" with the same vertical padding as the compose button,
/// so the two line up in a row.
pub fn add_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
    button(
        container(icon::plus().size(ICON_XL).style(text::primary))
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_DROPDOWN)
    .style(theme::ButtonClass::Action.style())
    .into()
}

pub fn settings_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
    button(
        container(
            row![
                container(icon::settings().size(ICON_LG).style(text::primary))
                    .align_y(Alignment::Center),
                container(
                    text(t!("main.sidebar.settings"))
                        .size(TEXT_LG)
                        .style(text::primary)
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill),
    )
    .on_press(on_press)
    .style(theme::ButtonClass::Experiment { variant: 10 }.style())
    .padding(PAD_BUTTON)
    .width(Length::Fill)
    .into()
}
