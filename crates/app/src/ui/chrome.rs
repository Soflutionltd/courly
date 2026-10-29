//! Main-window chrome in the Outlook layout: the brand-colored title bar
//! with the search field, the command ribbon, and the vertical app rail.

use cmdk::{BindingTable, CommandContext, CommandId, CommandRegistry};
use iced::widget::{Space, column, container, mouse_area, row};
use iced::{Alignment, Element, Length, Padding};

use crate::app::AppMode;
use crate::icon;
use crate::message::Message;
use crate::ui::layout::{
    APP_RAIL_WIDTH, PANE_GAP, SPACE_XS, SPACE_XXS, TITLE_BAR_HEIGHT, TITLE_SEARCH_MAX_WIDTH,
    TRAFFIC_LIGHTS_INSET,
};
use crate::ui::theme;
use crate::ui::widgets;

/// Empty title bar space that moves the window when pressed, since the
/// system title bar is transparent and covered by this one.
fn drag_area<'a>(width: Length) -> Element<'a, Message> {
    mouse_area(container(Space::new()).width(width).height(Length::Fill))
        .on_press(Message::DragMainWindow)
        .into()
}

pub fn title_bar<'a>(search: Element<'a, Message>) -> Element<'a, Message> {
    let left_inset = if cfg!(target_os = "macos") {
        TRAFFIC_LIGHTS_INSET
    } else {
        PANE_GAP
    };
    let search_slot = container(search)
        .width(Length::FillPortion(2).max(TITLE_SEARCH_MAX_WIDTH))
        .align_y(Alignment::Center);

    container(
        row![
            drag_area(Length::Fixed(left_inset)),
            drag_area(Length::FillPortion(1)),
            search_slot,
            drag_area(Length::FillPortion(1)),
        ]
        .height(Length::Fill)
        .align_y(Alignment::Center),
    )
    .height(TITLE_BAR_HEIGHT)
    .width(Length::Fill)
    .padding(Padding::from([SPACE_XXS, 0.0]))
    .style(theme::ContainerClass::TitleBar.style())
    .into()
}

pub fn ribbon<'a>(
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let cmd = |id: CommandId, glyph: iced::widget::Text<'a>| {
        widgets::command_icon_button(id, glyph, registry, binding_table, ctx)
    };

    let buttons = row![
        widgets::primary_icon_button(icon::pencil(), "New mail", Message::Compose),
        Space::new().width(SPACE_XS),
        cmd(CommandId::EmailTrash, icon::trash()),
        cmd(CommandId::EmailArchive, icon::archive()),
        cmd(CommandId::EmailSpam, icon::shield()),
        cmd(CommandId::EmailMoveToFolder, icon::folder()),
        widgets::ribbon_separator(),
        cmd(CommandId::EmailStar, icon::flag()),
        cmd(CommandId::EmailMarkRead, icon::mail()),
        cmd(CommandId::EmailSnooze, icon::clock()),
        widgets::ribbon_separator(),
        cmd(CommandId::ComposeReply, icon::reply()),
        cmd(CommandId::ComposeReplyAll, icon::reply_all()),
        cmd(CommandId::ComposeForward, icon::forward()),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    container(buttons)
        .padding(Padding::from([SPACE_XXS, SPACE_XS]))
        .width(Length::Fill)
        .style(theme::ContainerClass::Pane.style())
        .into()
}

pub fn app_rail<'a>(mode: AppMode, settings_open: bool) -> Element<'a, Message> {
    let in_mail = mode == AppMode::Mail && !settings_open;
    let in_calendar = mode == AppMode::Calendar && !settings_open;
    container(
        column![
            widgets::rail_button(icon::mail(), in_mail, Message::SetAppMode(AppMode::Mail)),
            widgets::rail_button(
                icon::calendar(),
                in_calendar,
                Message::SetAppMode(AppMode::Calendar)
            ),
            Space::new().height(Length::Fill),
            widgets::rail_button(icon::settings(), settings_open, Message::ToggleSettings),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center),
    )
    .width(APP_RAIL_WIDTH)
    .height(Length::Fill)
    .padding(Padding::from([PANE_GAP, 0.0]))
    .align_x(Alignment::Center)
    .into()
}
