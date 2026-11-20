//! The formatting controls of a rich text toolbar that open a menu or need
//! the format at the caret: font, size, text color, highlight, paragraph
//! style, and clearing. Shared by every surface that hosts the rich text
//! editor; each surface lays the controls out among its own buttons.

use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use iced::widget::{
    Row, Text, button, column, container, row, scrollable, text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Font, Length};
use rust_i18n::t;

use rte::{
    BlockAttrs, BlockKind, EditAction, FontFamily, FormatChange, HeadingLevel, Rgb, RunFormat,
    TableChange, TextAlignment,
};

use crate::icon;
use crate::ui::layout::{
    AVATAR_DROPDOWN_ITEM, DROPDOWN_ITEM_HEIGHT, FORMAT_OVERFLOW_PANEL_WIDTH, ICON_SM, PAD_BADGE,
    PAD_DROPDOWN, PAD_ICON_BTN, PAD_INPUT, PAD_NAV_ITEM, SELECT_MIN_WIDTH, SIDEBAR_MIN_WIDTH,
    SPACE_XS, SPACE_XXS, TEXT_MD, TEXT_SM, TEXT_XS, TYPEAHEAD_MAX_HEIGHT,
};
use crate::ui::theme;

use super::avatars::{color_dot_sized, radio_circle};
use super::overflow_row::overflow_row;

/// Which of the menus is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMenu {
    Color,
    Highlight,
    Font,
    Size,
    Block,
    Alignment,
    LineHeight,
    Table,
    /// The panel of the tools the bar had no room for.
    More,
}

/// What a formatting toolbar remembers between frames: the open menu, the
/// text typed in its field (a font search, a size, a color code), and how
/// many tools its last layout kept in the bar.
#[derive(Debug, Clone, Default)]
pub struct FormatBarState {
    open: Option<FormatMenu>,
    query: String,
    /// Written by the bar at each layout; the tools after it go in the
    /// "more" panel.
    shown: Arc<AtomicUsize>,
}

impl FormatBarState {
    /// Open `menu`, or close it if it is the open one.
    pub fn toggle(&mut self, menu: FormatMenu) {
        self.open = (self.open != Some(menu)).then_some(menu);
        self.query.clear();
    }

    /// Close whichever menu is open.
    pub fn close(&mut self) {
        self.open = None;
        self.query.clear();
    }

    /// Record the text typed in the open menu's field.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
    }
}

/// The controls built by [`format_bar`], for the host to order among its
/// own tools and hand to [`toolbar`].
pub struct FormatTools<'a, M> {
    pub font: Tool<'a, M>,
    pub size: Tool<'a, M>,
    pub smaller: Tool<'a, M>,
    pub larger: Tool<'a, M>,
    pub color: Tool<'a, M>,
    pub highlight: Tool<'a, M>,
    pub block: Tool<'a, M>,
    pub alignment: Tool<'a, M>,
    pub line_height: Tool<'a, M>,
    pub outdent: Tool<'a, M>,
    pub indent: Tool<'a, M>,
    pub table: Tool<'a, M>,
    pub clear: Tool<'a, M>,
}

/// One control of a formatting toolbar: how it shows in the bar, how it
/// shows in the "more" panel once the bar has no room for it, and its menu
/// when open.
pub struct Tool<'a, M> {
    bar: Element<'a, M>,
    panel: Element<'a, M>,
    menu: Option<ToolMenu<'a, M>>,
}

/// The open menu of a tool: under the tool, or under the "more" button
/// when the tool sits in the panel.
struct ToolMenu<'a, M> {
    content: Element<'a, M>,
    width: Option<f32>,
    on_dismiss: M,
}

impl<'a, M: Clone + 'a> Tool<'a, M> {
    /// An icon button named `label` in its tooltip.
    pub fn icon(glyph: impl Fn() -> Text<'a>, label: Cow<'static, str>, on_press: M) -> Self {
        Self::button(
            || glyph().size(ICON_SM).style(text::secondary).into(),
            label,
            on_press,
        )
    }

    /// A button showing what `content` draws, named `label` in its tooltip.
    fn button(content: impl Fn() -> Element<'a, M>, label: Cow<'static, str>, on_press: M) -> Self {
        Self {
            bar: with_tooltip(trigger(content(), on_press.clone()), label.clone()),
            panel: with_tooltip(trigger(content(), on_press), label),
            menu: None,
        }
    }

    /// Show `content` as this tool's open menu, `width` wide (the width of
    /// what it hangs under by default). A click away sends `on_dismiss`.
    pub fn with_menu(mut self, content: Element<'a, M>, width: Option<f32>, on_dismiss: M) -> Self {
        self.menu = Some(ToolMenu {
            content,
            width,
            on_dismiss,
        });
        self
    }
}

fn with_tooltip<'a, M: 'a>(content: Element<'a, M>, label: Cow<'static, str>) -> Element<'a, M> {
    tooltip(
        content,
        container(
            text(label)
                .size(TEXT_XS)
                .style(theme::TextClass::OnPrimary.style()),
        )
        .padding(PAD_BADGE)
        .style(theme::ContainerClass::Floating.style()),
        tooltip::Position::Bottom,
    )
    .into()
}

/// Lay `tools` out on one line, most important first: the ones the width
/// cannot hold fold, from the end, into a "more" panel, as in Outlook.
pub fn toolbar<'a, M: Clone + 'a>(
    tools: Vec<Tool<'a, M>>,
    state: &FormatBarState,
    on_toggle: impl Fn(FormatMenu) -> M,
) -> Element<'a, M> {
    let shown = state.shown.load(Ordering::Relaxed);
    let mut bars = Vec::with_capacity(tools.len());
    let mut panel = Vec::new();
    let mut folded_menu = None;
    for (index, tool) in tools.into_iter().enumerate() {
        if index < shown {
            bars.push(match tool.menu {
                Some(menu) => anchored(tool.bar, Some(menu.content), menu.width, menu.on_dismiss),
                None => tool.bar,
            });
        } else {
            bars.push(tool.bar);
            panel.push(tool.panel);
            if tool.menu.is_some() {
                folded_menu = tool.menu;
            }
        }
    }

    let more_button = with_tooltip(
        trigger(
            icon::ellipsis().size(ICON_SM).style(text::secondary).into(),
            on_toggle(FormatMenu::More),
        ),
        t!("compose.format.more"),
    );
    let more = match folded_menu {
        Some(menu) => anchored(more_button, Some(menu.content), menu.width, menu.on_dismiss),
        None if state.open == Some(FormatMenu::More) => anchored(
            more_button,
            Some(
                container(Row::with_children(panel).spacing(SPACE_XXS).wrap())
                    .padding(PAD_DROPDOWN)
                    .style(theme::ContainerClass::SelectMenu.style())
                    .width(Length::Fill)
                    .into(),
            ),
            Some(FORMAT_OVERFLOW_PANEL_WIDTH),
            on_toggle(FormatMenu::More),
        ),
        None => more_button,
    };

    overflow_row(bars, more, SPACE_XXS, Arc::clone(&state.shown)).into()
}

/// Largest image file taken into a message or a signature (bytes): the
/// image travels inside every mail that carries it.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Widest an image is when it enters the document (px).
const MAX_INSERTED_WIDTH: u32 = 600;

/// Let the user pick an image file and turn it into the edit that puts it
/// in the document. `Ok(None)` when the picker is cancelled.
pub async fn pick_image() -> Result<Option<EditAction>, String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter(
            t!("compose.format.image"),
            &["png", "jpg", "jpeg", "gif", "webp"],
        )
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let name = file.file_name();
    let bytes = file.read().await;
    image_action(&name, &bytes).map(Some)
}

/// The edit inserting the image file `name` holding `bytes`, at its own
/// size or scaled down to [`MAX_INSERTED_WIDTH`].
fn image_action(name: &str, bytes: &[u8]) -> Result<EditAction, String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(t!("compose.format.image_too_large", name = name).into_owned());
    }
    let unreadable = || t!("compose.format.image_unreadable", name = name).into_owned();
    let (width, height) = rte::image::pixel_size(bytes).ok_or_else(unreadable)?;
    let mime = mime_guess::from_path(name)
        .first_raw()
        .filter(|mime| mime.starts_with("image/"))
        .ok_or_else(unreadable)?;
    let (width, height) = if width > MAX_INSERTED_WIDTH {
        let scaled = u64::from(height) * u64::from(MAX_INSERTED_WIDTH) / u64::from(width);
        (
            MAX_INSERTED_WIDTH,
            u32::try_from(scaled).map_err(|_| unreadable())?.max(1),
        )
    } else {
        (width, height)
    };
    Ok(EditAction::InsertImage {
        src: rte::image::data_uri(mime, bytes),
        alt: String::new(),
        width: Some(width),
        height: Some(height),
    })
}

/// The line heights listed, in percent of the font size.
const LINE_HEIGHTS: [u16; 5] = [100, 115, 150, 200, 300];

/// The text colors on offer, six per row.
const TEXT_COLORS: [Rgb; 18] = [
    Rgb::new(0x00, 0x00, 0x00),
    Rgb::new(0x40, 0x40, 0x40),
    Rgb::new(0x6b, 0x6b, 0x6b),
    Rgb::new(0x9e, 0x9e, 0x9e),
    Rgb::new(0xff, 0xff, 0xff),
    Rgb::new(0x79, 0x55, 0x48),
    Rgb::new(0xc0, 0x39, 0x2b),
    Rgb::new(0xe5, 0x39, 0x35),
    Rgb::new(0xd3, 0x54, 0x00),
    Rgb::new(0xb7, 0x95, 0x0b),
    Rgb::new(0x1e, 0x84, 0x49),
    Rgb::new(0x11, 0x7a, 0x65),
    Rgb::new(0x1f, 0x61, 0x8d),
    Rgb::new(0x1e, 0x88, 0xe5),
    Rgb::new(0x2e, 0x40, 0x9f),
    Rgb::new(0x6c, 0x34, 0x83),
    Rgb::new(0xa9, 0x32, 0x6f),
    Rgb::new(0xd8, 0x1b, 0x60),
];

/// The highlight colors on offer: pale, so dark text stays readable.
const HIGHLIGHT_COLORS: [Rgb; 12] = [
    Rgb::new(0xff, 0xf5, 0x9d),
    Rgb::new(0xff, 0xe0, 0x82),
    Rgb::new(0xff, 0xcc, 0x80),
    Rgb::new(0xff, 0xab, 0x91),
    Rgb::new(0xf8, 0xbb, 0xd0),
    Rgb::new(0xe1, 0xbe, 0xe7),
    Rgb::new(0xc5, 0xca, 0xe9),
    Rgb::new(0xbb, 0xde, 0xfb),
    Rgb::new(0xb2, 0xeb, 0xf2),
    Rgb::new(0xc8, 0xe6, 0xc9),
    Rgb::new(0xdc, 0xed, 0xc8),
    Rgb::new(0xe0, 0xe0, 0xe0),
];

const COLORS_PER_ROW: usize = 6;

/// The text sizes listed as shortcuts, in CSS pixels. Any size in
/// [`rte::RUN_SIZE_RANGE`] can be typed.
const TEXT_SIZES: [u16; 18] = [
    8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 32, 36, 48, 60, 72, 96, 144,
];

/// Above this many matches, the installed fonts are listed in the interface
/// font: laying out hundreds of names each in its own face would load every
/// font on the machine.
const FONT_PREVIEW_LIMIT: usize = 40;

/// The icon fonts the app loads: glyphs, not text faces.
const ICON_FONT_FAMILIES: [&str; 2] = ["lucide", "Iced-Icons"];

/// Every font family installed on this machine, sorted by name. Read once:
/// a font installed while the app runs shows after a restart.
fn installed_font_families() -> &'static [FontFamily] {
    static FAMILIES: OnceLock<Vec<FontFamily>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let Ok(font_system) = iced::advanced::graphics::text::font_system().read() else {
            return Vec::new();
        };
        let mut families: Vec<FontFamily> = font_system
            .families()
            // A leading dot marks a face the system keeps for itself.
            .filter(|name| !name.starts_with('.'))
            .filter(|name| !ICON_FONT_FAMILIES.contains(name))
            .filter_map(FontFamily::new)
            .collect();
        families.sort_by_key(|family| family.name().to_lowercase());
        families.dedup();
        families
    })
}

/// The size text takes when its format leaves the size automatic.
fn automatic_size(block: Option<BlockKind>) -> u16 {
    match block {
        Some(BlockKind::Heading(HeadingLevel::H1)) => 18,
        Some(BlockKind::Heading(HeadingLevel::H2)) => 16,
        Some(BlockKind::Heading(HeadingLevel::H3)) => 14,
        _ => 13,
    }
}

/// What the caret of an editor is in, shown on the triggers of its toolbar
/// and marked in its menus.
#[derive(Debug, Clone, Copy)]
pub struct CaretContext {
    /// The color, font and size at the caret.
    pub format: RunFormat,
    /// The kind of the block holding the caret.
    pub block: Option<BlockKind>,
    /// That block's alignment, indent and line height.
    pub attrs: BlockAttrs,
    /// Whether the caret is in a table cell.
    pub in_table: bool,
}

impl CaretContext {
    /// Where the caret of `editor` is.
    pub fn of(editor: &rte::EditorState) -> Self {
        Self {
            format: editor.current_format(),
            block: editor.current_block_kind(),
            attrs: editor.current_block_attrs(),
            in_table: !editor.scope().is_empty(),
        }
    }
}

/// The formatting controls of a toolbar, for the caret `caret` describes.
pub fn format_bar<'a, M: Clone + 'a>(
    state: &'a FormatBarState,
    caret: CaretContext,
    on_toggle: impl Fn(FormatMenu) -> M,
    on_query: impl Fn(String) -> M + 'a,
    on_edit: impl Fn(EditAction) -> M,
) -> FormatTools<'a, M> {
    let CaretContext {
        format: current,
        block,
        attrs,
        in_table,
    } = caret;
    let set = |change: FormatChange| on_edit(EditAction::SetFormat(change));
    let query = state.query.as_str();

    let mut open_menu = match state.open {
        Some(FormatMenu::Font) => Some(font_menu(query, current, on_query, &set)),
        Some(FormatMenu::Size) => Some(size_menu(query, current, on_query, &set)),
        Some(FormatMenu::Color) => Some(color_menu(
            &TEXT_COLORS,
            query,
            current.color,
            on_query,
            |color| set(FormatChange::Color(color)),
        )),
        Some(FormatMenu::Highlight) => Some(color_menu(
            &HIGHLIGHT_COLORS,
            query,
            current.background,
            on_query,
            |color| set(FormatChange::Background(color)),
        )),
        Some(FormatMenu::Block) => Some(block_menu(block, &on_edit)),
        Some(FormatMenu::Alignment) => Some(alignment_menu(attrs.alignment, &on_edit)),
        Some(FormatMenu::LineHeight) => Some(line_height_menu(attrs.line_height, &on_edit)),
        Some(FormatMenu::Table) => Some(table_menu(in_table, &on_edit)),
        Some(FormatMenu::More) | None => None,
    };
    // A tool that opens `menu`, drawn by `content`; it carries the menu's
    // content while that menu is open.
    let mut menu_tool = |menu: FormatMenu,
                         content: &dyn Fn() -> Element<'a, M>,
                         label: Cow<'static, str>,
                         width: f32| {
        let tool = Tool::button(content, label, on_toggle(menu));
        match open_menu.take_if(|_| state.open == Some(menu)) {
            Some(content) => tool.with_menu(content, Some(width), on_toggle(menu)),
            None => tool,
        }
    };

    let font_label = current.font.map_or_else(
        || t!("compose.format.font").into_owned(),
        |font| font.name().to_owned(),
    );
    let size_label = current.size.map_or_else(
        || t!("compose.format.size").into_owned(),
        |size| size.to_string(),
    );
    let font = menu_tool(
        FormatMenu::Font,
        &|| label_with_chevron(font_label.clone()),
        t!("compose.format.font"),
        SIDEBAR_MIN_WIDTH,
    );
    let size = menu_tool(
        FormatMenu::Size,
        &|| label_with_chevron(size_label.clone()),
        t!("compose.format.size"),
        SELECT_MIN_WIDTH,
    );
    let color = menu_tool(
        FormatMenu::Color,
        &|| icon_with_chevron(icon::baseline()),
        t!("compose.format.text_color"),
        SIDEBAR_MIN_WIDTH,
    );
    let highlight = menu_tool(
        FormatMenu::Highlight,
        &|| icon_with_chevron(icon::highlighter()),
        t!("compose.format.highlight"),
        SIDEBAR_MIN_WIDTH,
    );
    let block_tool = menu_tool(
        FormatMenu::Block,
        &|| icon_with_chevron(icon::heading()),
        t!("compose.format.paragraph_style"),
        SELECT_MIN_WIDTH,
    );
    let alignment = menu_tool(
        FormatMenu::Alignment,
        &|| icon_with_chevron(alignment_icon(attrs.alignment)),
        t!("compose.format.alignment"),
        SELECT_MIN_WIDTH,
    );
    let line_height = menu_tool(
        FormatMenu::LineHeight,
        &|| icon_with_chevron(icon::line_height()),
        t!("compose.format.line_height"),
        SELECT_MIN_WIDTH,
    );
    let table = menu_tool(
        FormatMenu::Table,
        &|| icon_with_chevron(icon::table()),
        t!("compose.format.table"),
        SIDEBAR_MIN_WIDTH,
    );

    let shown_size = current.size.unwrap_or_else(|| automatic_size(block));
    let step = |size: u16| set(FormatChange::Size(Some(rte::clamp_run_size(size))));

    FormatTools {
        font,
        size,
        smaller: Tool::icon(
            icon::minus,
            t!("compose.format.smaller"),
            step(shown_size.saturating_sub(1)),
        ),
        larger: Tool::icon(
            icon::plus,
            t!("compose.format.larger"),
            step(shown_size.saturating_add(1)),
        ),
        color,
        highlight,
        block: block_tool,
        alignment,
        line_height,
        outdent: Tool::icon(
            icon::indent_decrease,
            t!("compose.format.outdent"),
            on_edit(EditAction::DecreaseIndent),
        ),
        indent: Tool::icon(
            icon::indent_increase,
            t!("compose.format.indent"),
            on_edit(EditAction::IncreaseIndent),
        ),
        table,
        clear: Tool::icon(
            icon::remove_formatting,
            t!("compose.format.clear"),
            on_edit(EditAction::ClearFormatting),
        ),
    }
}

/// The alignment control's icon: the alignment at the caret.
fn alignment_icon<'a>(alignment: TextAlignment) -> Text<'a> {
    match alignment {
        TextAlignment::Left => icon::align_left(),
        TextAlignment::Center => icon::align_center(),
        TextAlignment::Right => icon::align_right(),
        TextAlignment::Justify => icon::align_justify(),
    }
}

fn icon_with_chevron<'a, M: 'a>(glyph: Text<'a>) -> Element<'a, M> {
    row![
        glyph.size(ICON_SM).style(text::secondary),
        icon::chevron_down()
            .size(ICON_SM)
            .style(theme::TextClass::Tertiary.style()),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center)
    .into()
}

fn trigger<'a, M: Clone + 'a>(content: Element<'a, M>, on_press: M) -> Element<'a, M> {
    button(content)
        .on_press(on_press)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::BareIcon.style())
        .into()
}

fn label_with_chevron<'a, M: 'a>(label: String) -> Element<'a, M> {
    row![
        text(label).size(TEXT_SM).style(text::secondary),
        icon::chevron_down()
            .size(ICON_SM)
            .style(theme::TextClass::Tertiary.style()),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center)
    .into()
}

/// The trigger with its menu dropped beneath it when open. Clicking away
/// sends `on_dismiss`, the same message as the trigger: it toggles.
fn anchored<'a, M: Clone + 'a>(
    trigger: Element<'a, M>,
    menu: Option<Element<'a, M>>,
    menu_width: Option<f32>,
    on_dismiss: M,
) -> Element<'a, M> {
    let Some(menu) = menu else {
        return trigger;
    };
    let overlay = crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .on_dismiss(on_dismiss);
    match menu_width {
        Some(width) => overlay.popup_width(width).into(),
        None => overlay.into(),
    }
}

/// A menu frame around `header` (a field, or nothing) and a list that
/// scrolls once it is taller than a typeahead.
fn menu<'a, M: Clone + 'a>(
    header: Option<Element<'a, M>>,
    items: Vec<Element<'a, M>>,
) -> Element<'a, M> {
    let list = container(
        scrollable(column(items).spacing(SPACE_XXS).width(Length::Fill)).height(Length::Shrink),
    )
    .height(Length::Fit.max(TYPEAHEAD_MAX_HEIGHT))
    .width(Length::Fill);
    let mut content = column![].spacing(SPACE_XS).width(Length::Fill);
    if let Some(header) = header {
        content = content.push(header);
    }
    container(content.push(list))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .width(Length::Fill)
        .into()
}

/// The field at the top of a menu. Enter sends `on_submit` when the typed
/// text resolves to something.
fn menu_field<'a, M: Clone + 'a>(
    placeholder: Cow<'static, str>,
    query: &'a str,
    on_query: impl Fn(String) -> M + 'a,
    on_submit: Option<M>,
) -> Element<'a, M> {
    let mut field = text_input(placeholder, query)
        .on_input(on_query)
        .size(TEXT_MD)
        .padding(PAD_INPUT);
    if let Some(on_submit) = on_submit {
        field = field.on_submit(on_submit);
    }
    field.into()
}

fn menu_item<'a, M: Clone + 'a>(
    label: String,
    font: Option<Font>,
    selected: bool,
    on_press: M,
) -> Element<'a, M> {
    let mut label = text(label).size(TEXT_MD).style(text::base);
    if let Some(font) = font {
        label = label.font(font);
    }
    button(
        row![
            radio_circle(selected),
            container(label)
                .width(Length::Fill)
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Dropdown { selected }.style())
    .width(Length::Fill)
    .into()
}

fn section_label<'a, M: 'a>(label: String) -> Element<'a, M> {
    container(
        text(label)
            .size(TEXT_XS)
            .style(theme::TextClass::Tertiary.style()),
    )
    .padding(PAD_NAV_ITEM)
    .into()
}

/// A search field, "Automatic", the families a reader is likely to have,
/// then every other installed family.
fn font_menu<'a, M: Clone + 'a>(
    query: &'a str,
    current: RunFormat,
    on_query: impl Fn(String) -> M + 'a,
    set: &impl Fn(FormatChange) -> M,
) -> Element<'a, M> {
    let installed = installed_font_families();
    let needle = query.trim().to_lowercase();
    let matches = |family: &FontFamily| family.name().to_lowercase().contains(&needle);
    let is_safe = |family: &FontFamily| FontFamily::SAFE.iter().any(|(safe, _)| safe == family);

    // A listed family this machine lacks would preview in another face, so
    // it is left out; with no font list at all, the whole list is offered.
    let safe: Vec<FontFamily> = FontFamily::SAFE
        .iter()
        .map(|(family, _)| *family)
        .filter(|family| installed.is_empty() || installed.contains(family))
        .filter(|family| matches(family))
        .collect();
    let others: Vec<FontFamily> = installed
        .iter()
        .copied()
        .filter(|family| !is_safe(family) && matches(family))
        .collect();

    let item = |family: FontFamily, preview: bool| {
        menu_item(
            family.name().to_owned(),
            preview.then(|| Font::new(family.name())),
            current.font == Some(family),
            set(FormatChange::Font(Some(family))),
        )
    };

    let mut items = Vec::with_capacity(safe.len() + others.len() + 3);
    if needle.is_empty() {
        items.push(menu_item(
            t!("compose.format.automatic").into_owned(),
            None,
            current.font.is_none(),
            set(FormatChange::Font(None)),
        ));
    }
    if !safe.is_empty() {
        items.push(section_label(
            t!("compose.format.fonts_common").into_owned(),
        ));
        items.extend(safe.iter().map(|family| item(*family, true)));
    }
    if !others.is_empty() {
        items.push(section_label(t!("compose.format.fonts_all").into_owned()));
        let preview = others.len() <= FONT_PREVIEW_LIMIT;
        items.extend(others.iter().map(|family| item(*family, preview)));
    }
    if safe.is_empty() && others.is_empty() {
        items.push(section_label(
            t!("compose.format.no_font_match").into_owned(),
        ));
    }

    let first_match = safe
        .first()
        .or(others.first())
        .filter(|_| !needle.is_empty())
        .map(|family| set(FormatChange::Font(Some(*family))));
    let field = menu_field(
        t!("compose.format.search_font"),
        query,
        on_query,
        first_match,
    );
    menu(Some(field), items)
}

/// A field taking any size in the range, "Automatic", then the shortcuts.
fn size_menu<'a, M: Clone + 'a>(
    query: &'a str,
    current: RunFormat,
    on_query: impl Fn(String) -> M + 'a,
    set: &impl Fn(FormatChange) -> M,
) -> Element<'a, M> {
    let typed = query
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|size| rte::RUN_SIZE_RANGE.contains(size))
        .map(|size| set(FormatChange::Size(Some(size))));
    let field = menu_field(
        t!(
            "compose.format.size_range",
            min = rte::RUN_SIZE_RANGE.start(),
            max = rte::RUN_SIZE_RANGE.end()
        ),
        query,
        on_query,
        typed,
    );

    let mut items = vec![menu_item(
        t!("compose.format.automatic").into_owned(),
        None,
        current.size.is_none(),
        set(FormatChange::Size(None)),
    )];
    items.extend(TEXT_SIZES.into_iter().map(|size| {
        menu_item(
            size.to_string(),
            None,
            current.size == Some(size),
            set(FormatChange::Size(Some(size))),
        )
    }));
    menu(Some(field), items)
}

/// "Automatic", the swatches in rows, then a field taking a color code.
fn color_menu<'a, M: Clone + 'a>(
    palette: &[Rgb],
    query: &'a str,
    current: Option<Rgb>,
    on_query: impl Fn(String) -> M + 'a,
    choose: impl Fn(Option<Rgb>) -> M,
) -> Element<'a, M> {
    let mut items = vec![menu_item(
        t!("compose.format.automatic").into_owned(),
        None,
        current.is_none(),
        choose(None),
    )];
    for colors in palette.chunks(COLORS_PER_ROW) {
        let swatches = colors.iter().map(|&color| {
            let selected = current == Some(color);
            button(color_dot_sized(
                Color::from_rgb8(color.r, color.g, color.b),
                AVATAR_DROPDOWN_ITEM,
            ))
            .on_press(choose(Some(color)))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Dropdown { selected }.style())
            .into()
        });
        items.push(Row::with_children(swatches).spacing(SPACE_XXS).into());
    }

    let typed = typed_color(query).map(|color| choose(Some(color)));
    let field = menu_field(t!("compose.format.color_code"), query, on_query, typed);
    menu(Some(field), items)
}

/// A color code as typed: `#rgb` or `#rrggbb`, with or without the `#`.
fn typed_color(query: &str) -> Option<Rgb> {
    let code = query.trim();
    if code.starts_with('#') {
        Rgb::parse_css(code)
    } else {
        Rgb::parse_css(&format!("#{code}"))
    }
}

fn block_label(kind: BlockKind) -> String {
    match kind {
        BlockKind::Heading(level) => {
            t!("compose.format.heading", level = level.as_u8()).into_owned()
        }
        _ => t!("compose.format.normal_text").into_owned(),
    }
}

/// Normal text and the three heading levels.
fn block_menu<'a, M: Clone + 'a>(
    current: Option<BlockKind>,
    on_edit: &impl Fn(EditAction) -> M,
) -> Element<'a, M> {
    let is_heading = matches!(current, Some(BlockKind::Heading(_)));
    let mut items = vec![menu_item(
        block_label(BlockKind::Paragraph),
        None,
        !is_heading,
        on_edit(EditAction::SetBlockType(BlockKind::Paragraph)),
    )];
    for level in [HeadingLevel::H1, HeadingLevel::H2, HeadingLevel::H3] {
        let kind = BlockKind::Heading(level);
        // Choosing the level a heading already has turns it back into
        // normal text: the editor's rule for a block type is a toggle.
        items.push(menu_item(
            block_label(kind),
            None,
            current == Some(kind),
            on_edit(EditAction::SetBlockType(kind)),
        ));
    }
    menu(None, items)
}

fn alignment_label(alignment: TextAlignment) -> String {
    match alignment {
        TextAlignment::Left => t!("compose.format.align_left"),
        TextAlignment::Center => t!("compose.format.align_center"),
        TextAlignment::Right => t!("compose.format.align_right"),
        TextAlignment::Justify => t!("compose.format.align_justify"),
    }
    .into_owned()
}

/// Left, centered, right and justified.
fn alignment_menu<'a, M: Clone + 'a>(
    current: TextAlignment,
    on_edit: &impl Fn(EditAction) -> M,
) -> Element<'a, M> {
    let items = [
        TextAlignment::Left,
        TextAlignment::Center,
        TextAlignment::Right,
        TextAlignment::Justify,
    ]
    .into_iter()
    .map(|alignment| {
        menu_item(
            alignment_label(alignment),
            None,
            current == alignment,
            on_edit(EditAction::SetAlignment(alignment)),
        )
    })
    .collect();
    menu(None, items)
}

/// A line height in percent as a multiplier: 115 reads "1.15".
fn line_height_label(percent: u16) -> String {
    format!("{}.{:02}", percent / 100, percent % 100)
}

/// "Automatic", then the listed line heights.
/// The sizes a table can be inserted with, as `(rows, columns)`.
const TABLE_SIZES: [(usize, usize); 6] = [(1, 2), (2, 2), (2, 3), (3, 3), (3, 4), (4, 4)];

/// The table menu: what can be done to the table holding the caret, when
/// it is in one, then the tables that can be inserted.
fn table_menu<'a, M: Clone + 'a>(
    in_table: bool,
    on_edit: &impl Fn(EditAction) -> M,
) -> Element<'a, M> {
    let mut items = Vec::new();
    if in_table {
        let changes = [
            (
                t!("compose.format.table_row_above"),
                TableChange::InsertRowAbove,
            ),
            (
                t!("compose.format.table_row_below"),
                TableChange::InsertRowBelow,
            ),
            (
                t!("compose.format.table_column_left"),
                TableChange::InsertColumnLeft,
            ),
            (
                t!("compose.format.table_column_right"),
                TableChange::InsertColumnRight,
            ),
            (
                t!("compose.format.table_delete_row"),
                TableChange::DeleteRow,
            ),
            (
                t!("compose.format.table_delete_column"),
                TableChange::DeleteColumn,
            ),
            (t!("compose.format.table_delete"), TableChange::DeleteTable),
        ];
        items.extend(changes.into_iter().map(|(label, change)| {
            menu_item(
                label.into_owned(),
                None,
                false,
                on_edit(EditAction::Table(change)),
            )
        }));
    }
    items.extend(TABLE_SIZES.into_iter().map(|(rows, columns)| {
        menu_item(
            t!(
                "compose.format.table_insert",
                rows = rows,
                columns = columns
            )
            .into_owned(),
            None,
            false,
            on_edit(EditAction::InsertTable { rows, columns }),
        )
    }));
    menu(None, items)
}

fn line_height_menu<'a, M: Clone + 'a>(
    current: Option<u16>,
    on_edit: &impl Fn(EditAction) -> M,
) -> Element<'a, M> {
    let mut items = vec![menu_item(
        t!("compose.format.automatic").into_owned(),
        None,
        current.is_none(),
        on_edit(EditAction::SetLineHeight(None)),
    )];
    items.extend(LINE_HEIGHTS.into_iter().map(|percent| {
        menu_item(
            line_height_label(percent),
            None,
            current == Some(percent),
            on_edit(EditAction::SetLineHeight(Some(percent))),
        )
    }));
    menu(None, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header of a PNG of the given size, enough to read its dimensions.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png
    }

    #[test]
    fn a_picked_image_enters_at_its_size_or_scaled_to_the_widest_allowed() {
        let small = image_action("logo.png", &png_header(300, 120));
        let Ok(EditAction::InsertImage {
            src, width, height, ..
        }) = small
        else {
            panic!("a PNG is inserted");
        };
        assert!(src.starts_with("data:image/png;base64,"));
        assert_eq!((width, height), (Some(300), Some(120)));

        let wide = image_action("photo.png", &png_header(2400, 1200));
        assert!(matches!(
            wide,
            Ok(EditAction::InsertImage {
                width: Some(600),
                height: Some(300),
                ..
            })
        ));
    }

    #[test]
    fn a_file_that_is_not_an_image_or_is_too_heavy_is_refused() {
        assert!(image_action("notes.png", b"plain text").is_err());
        assert!(image_action("logo.txt", &png_header(300, 120)).is_err());
        let mut heavy = png_header(300, 120);
        heavy.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(image_action("logo.png", &heavy).is_err());
    }

    #[test]
    fn a_color_code_is_read_with_or_without_its_hash() {
        assert_eq!(typed_color("#ff8800"), Some(Rgb::new(0xff, 0x88, 0x00)));
        assert_eq!(typed_color(" F80 "), Some(Rgb::new(0xff, 0x88, 0x00)));
        assert_eq!(typed_color("orange"), None);
        assert_eq!(typed_color(""), None);
    }

    #[test]
    fn line_heights_read_as_multipliers_and_fit_the_range() {
        assert_eq!(line_height_label(100), "1.00");
        assert_eq!(line_height_label(115), "1.15");
        assert!(
            LINE_HEIGHTS
                .iter()
                .all(|percent| rte::LINE_HEIGHT_RANGE.contains(percent))
        );
    }

    #[test]
    fn the_size_shortcuts_are_all_sizes_a_run_can_carry() {
        assert!(
            TEXT_SIZES
                .iter()
                .all(|size| rte::RUN_SIZE_RANGE.contains(size))
        );
    }
}
