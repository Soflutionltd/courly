use cmdk::{CommandContext, CommandRegistry};
use iced::widget::{
    Space, button, checkbox, column, container, mouse_area, responsive, row, scrollable, text,
};
use iced::{Alignment, Element, Length, Padding, Size, Vector, mouse};
use rust_i18n::t;

use super::{DragSource, DropTarget, LabelMode, Toolbar, ToolbarItem, ToolbarMenu, ToolbarMessage};
use crate::icon;
use crate::message::Message;
use crate::ui::anchored_overlay::{AnchorPosition, AnchoredOverlay, anchored_overlay};
use crate::ui::layout::*;
use crate::ui::move_menu::{MoveMenu, MoveMenuMessage};
use crate::ui::side_panel::SidePanelMessage;
use crate::ui::theme;
use crate::ui::widgets;

/// One bar button, resolved against the current selection.
#[derive(Debug, Clone)]
struct Resolved {
    item: ToolbarItem,
    index: usize,
    label: String,
    available: bool,
}

fn resolve(state: &Toolbar, registry: &CommandRegistry, ctx: &CommandContext) -> Vec<Resolved> {
    state
        .layout
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, &item)| {
            let desc = registry.get(item.command())?;
            Some(Resolved {
                item,
                index,
                label: crate::i18n::command(desc.resolved_label(ctx)).into_owned(),
                available: (desc.is_available)(ctx),
            })
        })
        .collect()
}

/// The neutral name of a button ("Mark Read" rather than its current
/// toggle state), for the customize panel and the dragged button.
fn catalog_label(registry: &CommandRegistry, item: ToolbarItem) -> String {
    registry
        .get(item.command())
        .map(|desc| crate::i18n::command(desc.label).into_owned())
        .unwrap_or_default()
}

fn toolbar_message(message: ToolbarMessage) -> Message {
    Message::Toolbar(message)
}

pub fn toolbar<'a>(
    state: &'a Toolbar,
    registry: &CommandRegistry,
    ctx: &CommandContext,
    side_panel_open: bool,
    move_menu: Option<&'a MoveMenu>,
) -> Element<'a, Message> {
    let buttons = resolve(state, registry, ctx);
    let labels = state.layout.labels;
    let bar = responsive(move |size| {
        if state.customizing {
            editable_bar(state, &buttons, labels, size)
        } else {
            live_bar(state, &buttons, labels, size, move_menu)
        }
    })
    .height(Length::Shrink);

    let content = row![
        widgets::primary_icon_button(
            icon::pencil(),
            t!("main.ribbon.new_mail"),
            Message::Compose
        ),
        Space::new().width(SPACE_XS),
        bar,
        side_panel_toggle(side_panel_open),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let mut ribbon = anchored_overlay(
        container(content)
            .padding(Padding::from([SPACE_XXS, SPACE_XS]))
            .width(Length::Fill)
            .style(theme::ContainerClass::Pane.style()),
    )
    .on_right_press(|point| toolbar_message(ToolbarMessage::OpenBackgroundMenu(point)));
    if state.customizing {
        let catalog: Vec<(ToolbarItem, String)> = ToolbarItem::ALL
            .iter()
            .map(|&item| (item, catalog_label(registry, item)))
            .collect();
        ribbon = ribbon
            .popup(customize_panel(state, &catalog))
            .popup_width(TOOLBAR_CUSTOMIZE_WIDTH);
    } else if let Some(ToolbarMenu::Background(point)) = state.menu {
        ribbon = ribbon
            .popup(menu_card(vec![customize_entry()]))
            .anchor_point(point)
            .popup_width(TOOLBAR_MENU_WIDTH)
            .on_dismiss(toolbar_message(ToolbarMessage::CloseMenu));
    }

    let dragged = state.drag.as_ref().filter(|drag| drag.moved).and_then(|drag| {
        let item = match drag.source {
            DragSource::Bar(index) => state.layout.items.get(index).copied()?,
            DragSource::Catalog(item) => item,
        };
        Some((item, drag.cursor?))
    });
    match dragged {
        Some((item, cursor)) => anchored_overlay(ribbon)
            .popup(drag_ghost(item, &catalog_label(registry, item), labels))
            .anchor_point(cursor + Vector::new(SPACE_SM, SPACE_SM))
            .into(),
        None => ribbon.into(),
    }
}

/// The fixed button at the right end that opens and closes the side panel.
fn side_panel_toggle<'a>(open: bool) -> Element<'a, Message> {
    let ico = if open {
        icon::panel_right_close()
    } else {
        icon::panel_right_open()
    };
    widgets::nav_icon_button(ico, open, Message::SidePanel(SidePanelMessage::Toggle))
}

// ── Live bar ────────────────────────────────────────────

/// The buttons that fit, then "…" with the rest, the add-ins and
/// "Customize toolbar". The Move menu hangs under Move, or under "…" when
/// Move is not on the bar.
fn live_bar<'a>(
    state: &'a Toolbar,
    buttons: &[Resolved],
    labels: LabelMode,
    size: Size,
    move_menu: Option<&'a MoveMenu>,
) -> Element<'a, Message> {
    let room = size.width - more_button_width() - SPACE_XXS;
    let mut used = 0.0;
    let mut shown = 0;
    for resolved in buttons {
        let width = button_width(&resolved.label, labels) + if shown > 0 { SPACE_XXS } else { 0.0 };
        if used + width > room {
            break;
        }
        used += width;
        shown += 1;
    }

    let move_shown = buttons[..shown]
        .iter()
        .any(|resolved| resolved.item == ToolbarItem::Move);
    let mut bar = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    for resolved in &buttons[..shown] {
        let menu = move_menu.filter(|_| resolved.item == ToolbarItem::Move);
        bar = bar.push(bar_button(state, resolved, labels, menu));
    }
    let menu = move_menu.filter(|_| !move_shown);
    bar.push(more_button(state, &buttons[shown..], menu)).into()
}

fn with_move_menu<'a>(
    slot: AnchoredOverlay<'a, Message>,
    menu: &'a MoveMenu,
) -> AnchoredOverlay<'a, Message> {
    slot.popup(menu.view())
        .popup_width(MOVE_MENU_WIDTH)
        .on_dismiss(Message::MoveMenu(MoveMenuMessage::Close))
}

fn bar_button<'a>(
    state: &'a Toolbar,
    resolved: &Resolved,
    labels: LabelMode,
    move_menu: Option<&'a MoveMenu>,
) -> Element<'a, Message> {
    let mut face = button(button_face(
        resolved.item,
        &resolved.label,
        labels,
        resolved.available,
    ))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Action.style());
    if resolved.available {
        face = face.on_press(Message::ExecuteCommand(resolved.item.command()));
    }
    let face: Element<'a, Message> = face.into();

    let index = resolved.index;
    let mut slot = anchored_overlay(face).on_right_press_offset(move |offset| {
        toolbar_message(ToolbarMessage::OpenItemMenu { index, offset })
    });
    if let Some(ToolbarMenu::Item { index: open, offset }) = state.menu
        && open == index
    {
        slot = slot
            .popup(menu_card(vec![
                menu_row(
                    icon::trash(),
                    t!("main.toolbar.remove").into_owned(),
                    Some(toolbar_message(ToolbarMessage::Remove(index))),
                ),
                customize_entry(),
            ]))
            .anchor_offset(offset)
            .popup_width(TOOLBAR_MENU_WIDTH)
            .on_dismiss(toolbar_message(ToolbarMessage::CloseMenu));
    } else if let Some(menu) = move_menu {
        slot = with_move_menu(slot, menu);
    }
    slot.into()
}

fn more_button<'a>(
    state: &'a Toolbar,
    hidden: &[Resolved],
    move_menu: Option<&'a MoveMenu>,
) -> Element<'a, Message> {
    let face = button(icon::ellipsis().size(ICON_MD).style(text::secondary))
        .on_press(toolbar_message(ToolbarMessage::ToggleOverflow))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());
    if let Some(menu) = move_menu {
        return with_move_menu(anchored_overlay(face), menu)
            .position(AnchorPosition::BelowRight)
            .into();
    }
    if !state.overflow_open {
        return face.into();
    }
    let mut entries: Vec<Element<'a, Message>> = hidden
        .iter()
        .map(|resolved| {
            menu_row(
                resolved.item.icon(),
                resolved.label.clone(),
                resolved
                    .available
                    .then(|| toolbar_message(ToolbarMessage::RunFromOverflow(resolved.item.command()))),
            )
        })
        .collect();
    if !entries.is_empty() {
        entries.push(widgets::divider());
    }
    entries.push(customize_entry());
    anchored_overlay(face)
        .popup(menu_card(entries))
        .position(AnchorPosition::BelowRight)
        .popup_width(TOOLBAR_MENU_WIDTH)
        .on_dismiss(toolbar_message(ToolbarMessage::CloseOverflow))
        .into()
}

// ── Editable bar ────────────────────────────────────────

/// Every button as a drag handle, each one a drop slot, then an end slot
/// for appending. Scrolls sideways when the buttons don't all fit.
fn editable_bar<'a>(
    state: &'a Toolbar,
    buttons: &[Resolved],
    labels: LabelMode,
    size: Size,
) -> Element<'a, Message> {
    let drag = state.drag.as_ref().filter(|drag| drag.moved);
    let target = drag.and_then(|drag| drag.target);

    let mut bar = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    let mut total = 0.0;
    for resolved in buttons {
        let index = resolved.index;
        total += button_width(&resolved.label, labels) + SPACE_XXS;
        let class = if drag.is_some_and(|drag| drag.source == DragSource::Bar(index)) {
            Some(theme::ContainerClass::DraggingRow)
        } else if target == Some(DropTarget::Before(index)) {
            Some(theme::ContainerClass::DropTarget)
        } else {
            None
        };
        let mut face = container(button_face(resolved.item, &resolved.label, labels, true))
            .padding(PAD_ICON_BTN);
        if let Some(class) = class {
            face = face.style(class.style());
        }
        bar = bar.push(drop_slot(face, DropTarget::Before(index)).on_press(
            toolbar_message(ToolbarMessage::DragStart(DragSource::Bar(index))),
        ));
    }

    let overflows = total + TOOLBAR_END_SLOT_WIDTH > size.width;
    let mut end = container(Space::new())
        .width(if overflows {
            Length::Fixed(TOOLBAR_END_SLOT_WIDTH)
        } else {
            Length::Fill
        })
        .height(TOOLBAR_SLOT_HEIGHT);
    if target == Some(DropTarget::End) {
        end = end.style(theme::ContainerClass::DropTarget.style());
    }
    bar = bar.push(drop_slot(end, DropTarget::End));

    if overflows {
        scrollable(bar)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new()
                    .width(TOOLBAR_SCROLLBAR_WIDTH)
                    .scroller_width(TOOLBAR_SCROLLBAR_WIDTH)
                    .spacing(SCROLLBAR_SPACING),
            ))
            .width(Length::Fill)
            .into()
    } else {
        bar.width(Length::Fill).into()
    }
}

/// Reports the pointer entering and leaving a drop slot, and the drop.
fn drop_slot<'a>(
    content: impl Into<Element<'a, Message>>,
    target: DropTarget,
) -> iced::widget::MouseArea<'a, Message> {
    mouse_area(content)
        .on_enter(toolbar_message(ToolbarMessage::DragOver(target)))
        .on_exit(toolbar_message(ToolbarMessage::DragLeave(target)))
        .on_release(toolbar_message(ToolbarMessage::DragEnd))
        .interaction(mouse::Interaction::Grab)
}

// ── Customize panel ─────────────────────────────────────

fn customize_panel<'a>(
    state: &'a Toolbar,
    catalog: &[(ToolbarItem, String)],
) -> Element<'a, Message> {
    let mut grid = column![].spacing(SPACE_XXS);
    for chunk in catalog.chunks(TOOLBAR_CUSTOMIZE_COLUMNS) {
        let mut line = row![].spacing(SPACE_XXS);
        for (item, label) in chunk {
            line = line.push(catalog_tile(*item, label.clone()));
        }
        for _ in chunk.len()..TOOLBAR_CUSTOMIZE_COLUMNS {
            line = line.push(Space::new().width(Length::Fill));
        }
        grid = grid.push(line);
    }

    let always = state.layout.labels == LabelMode::Always;
    let footer = row![
        button(text(t!("main.toolbar.reset")).size(TEXT_MD))
            .on_press(toolbar_message(ToolbarMessage::Reset))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Secondary.style()),
        checkbox(always)
            .on_toggle(|on| {
                toolbar_message(ToolbarMessage::SetLabels(if on {
                    LabelMode::Always
                } else {
                    LabelMode::Never
                }))
            })
            .size(RADIO_SIZE),
        text(t!("main.toolbar.show_labels"))
            .size(TEXT_MD)
            .style(text::base),
        Space::new().width(Length::Fill),
        button(text(t!("main.toolbar.done")).size(TEXT_MD))
            .on_press(toolbar_message(ToolbarMessage::Done))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style()),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(
        column![
            text(t!("main.toolbar.customize_hint"))
                .size(TEXT_LG)
                .style(text::base),
            text(t!("main.toolbar.mail_options"))
                .size(TEXT_SM)
                .font(crate::font::text_semibold())
                .style(theme::TextClass::Tertiary.style()),
            grid,
            widgets::divider(),
            footer,
        ]
        .spacing(SPACE_SM),
    )
    .padding(SPACE_MD)
    .width(Length::Fill)
    .style(theme::ContainerClass::SelectMenu.style())
    .into()
}

fn catalog_tile<'a>(item: ToolbarItem, label: String) -> Element<'a, Message> {
    mouse_area(
        container(
            row![
                item.icon().size(ICON_MD).style(text::secondary),
                text(label).size(TEXT_MD).style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .padding(PAD_NAV_ITEM)
        .width(Length::Fill),
    )
    .on_press(toolbar_message(ToolbarMessage::DragStart(DragSource::Catalog(item))))
    .on_release(toolbar_message(ToolbarMessage::DragEnd))
    .interaction(mouse::Interaction::Grab)
    .into()
}

fn drag_ghost<'a>(item: ToolbarItem, label: &str, labels: LabelMode) -> Element<'a, Message> {
    container(button_face(item, label, labels, true))
        .padding(PAD_ICON_BTN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

// ── Shared pieces ───────────────────────────────────────

fn button_face<'a>(
    item: ToolbarItem,
    label: &str,
    labels: LabelMode,
    available: bool,
) -> Element<'a, Message> {
    let style: fn(&iced::Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };
    let mut face = row![item.icon().size(ICON_MD).style(style)]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center);
    if labels == LabelMode::Always {
        face = face.push(text(label.to_owned()).size(TEXT_SM).style(style));
    }
    face.into()
}

fn customize_entry<'a>() -> Element<'a, Message> {
    menu_row(
        icon::settings(),
        t!("main.toolbar.customize").into_owned(),
        Some(toolbar_message(ToolbarMessage::Customize)),
    )
}

fn menu_card<'a>(entries: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    container(column(entries).spacing(SPACE_XXXS))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

fn menu_row<'a>(
    glyph: iced::widget::Text<'a>,
    label: String,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let style: fn(&iced::Theme) -> text::Style = if on_press.is_some() {
        text::base
    } else {
        theme::TextClass::Tertiary.style()
    };
    button(
        container(
            row![
                glyph.size(ICON_MD).style(style),
                text(label).size(TEXT_MD).style(style),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press_maybe(on_press)
    .padding(ROW_MENU_ITEM_PADDING)
    .height(ROW_MENU_ITEM_HEIGHT)
    .width(Length::Fill)
    .style(theme::ButtonClass::Dropdown { selected: false }.style())
    .into()
}

// ── Measuring ───────────────────────────────────────────

fn measure(content: &str, size: f32, font: iced::Font) -> f32 {
    use iced::advanced::text::{
        Alignment, Ellipsis, LineHeight, Paragraph as ParagraphTrait, Shaping, Wrapping,
    };
    type IcedParagraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;

    IcedParagraph::with_text(iced::advanced::text::Text {
        content,
        bounds: Size::new(f32::INFINITY, f32::INFINITY),
        size: iced::Pixels(size),
        line_height: LineHeight::default(),
        font,
        align_x: Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
        ellipsis: Ellipsis::None,
        hint_factor: None,
    })
    .min_bounds()
    .width
}

/// Width of a bar button as laid out by [`bar_button`].
fn button_width(label: &str, labels: LabelMode) -> f32 {
    let glyph = ICON_MD;
    let name = match labels {
        LabelMode::Always => SPACE_XXS + measure(label, TEXT_SM, crate::font::text()),
        LabelMode::Never => 0.0,
    };
    PAD_ICON_BTN.left + glyph + name + PAD_ICON_BTN.right + TOOLBAR_MEASURE_SLACK
}

fn more_button_width() -> f32 {
    PAD_ICON_BTN.left + ICON_MD + PAD_ICON_BTN.right + TOOLBAR_MEASURE_SLACK
}
