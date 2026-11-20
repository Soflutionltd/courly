//! Templates: reusable messages stored on this computer. A click drops one
//! into the message being written, or starts a new message with it.

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};
use rte::{Action as RteAction, BlockKind, EditAction, InlineStyle, rich_text_editor};
use rust_i18n::t;

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

/// Characters of body text shown under a template's name.
const PREVIEW_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub subject: Option<String>,
    pub body_html: String,
    preview: String,
}

impl Template {
    pub fn new(id: String, name: String, subject: Option<String>, body_html: String) -> Self {
        let document = rte::html_parse::from_html(&body_html);
        let text: Vec<String> = document
            .blocks
            .iter()
            .map(|block| block.flattened_text())
            .filter(|line| !line.trim().is_empty())
            .collect();
        let preview = text.join(" ").chars().take(PREVIEW_CHARS).collect();
        Self {
            id,
            name,
            subject,
            body_html,
            preview,
        }
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.name.to_lowercase().contains(&query)
            || self
                .subject
                .as_deref()
                .is_some_and(|subject| subject.to_lowercase().contains(&query))
            || self.preview.to_lowercase().contains(&query)
    }
}

/// The template being written or changed.
pub struct Editor {
    pub id: Option<String>,
    pub name: String,
    pub subject: String,
    pub body: rte::EditorState,
    /// The color, font or size menu of the formatting bar that is open.
    pub format_bar: widgets::FormatBarState,
    pub saving: bool,
}

impl Editor {
    fn can_save(&self) -> bool {
        !self.saving && !self.name.trim().is_empty()
    }
}

#[derive(Default)]
pub struct Templates {
    pub templates: Vec<Template>,
    pub query: String,
    pub editor: Option<Editor>,
    pub confirm_delete: Option<String>,
}

impl Templates {
    pub fn find(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|template| template.id == id)
    }

    pub fn start_new(&mut self) {
        self.confirm_delete = None;
        self.editor = Some(Editor {
            id: None,
            name: String::new(),
            subject: String::new(),
            body: rte::EditorState::new(),
            format_bar: widgets::FormatBarState::default(),
            saving: false,
        });
    }

    pub fn start_edit(&mut self, id: &str) {
        let Some(template) = self.find(id) else {
            return;
        };
        let editor = Editor {
            id: Some(template.id.clone()),
            name: template.name.clone(),
            subject: template.subject.clone().unwrap_or_default(),
            body: rte::EditorState::from_html(&template.body_html),
            format_bar: widgets::FormatBarState::default(),
            saving: false,
        };
        self.confirm_delete = None;
        self.editor = Some(editor);
    }
}

#[derive(Debug, Clone)]
pub enum TemplatesMessage {
    Loaded(Result<Vec<Template>, String>),
    Query(String),
    New,
    Edit(String),
    Name(String),
    Subject(String),
    Body(RteAction),
    /// Open the color, font or size menu, or close it if it is the open one.
    ToggleFormatMenu(widgets::FormatMenu),
    /// Text typed in the open formatting menu's field.
    FormatQuery(String),
    Save,
    Saved(Result<String, String>),
    Cancel,
    Delete(String),
    ConfirmDelete(String),
    CancelDelete,
    Deleted(Result<(), String>),
    Insert(String),
}

pub fn view(state: &Templates) -> Element<'_, TemplatesMessage> {
    let content = match &state.editor {
        Some(editor) => editor_view(editor),
        None => list_view(state),
    };
    container(content).padding(PAD_SIDE_PANEL).into()
}

fn list_view(state: &Templates) -> Element<'_, TemplatesMessage> {
    let search = text_input(t!("main.side_panel.templates.search"), &state.query)
        .on_input(TemplatesMessage::Query)
        .padding(PAD_INPUT)
        .size(TEXT_SM);
    let header = row![
        search,
        widgets::nav_icon_button(icon::plus(), false, TemplatesMessage::New),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let mut list = column![header].spacing(SPACE_XS);
    if state.templates.is_empty() {
        return list
            .push(hint(t!("main.side_panel.templates.empty")))
            .push(
                button(text(t!("main.side_panel.templates.new")).size(TEXT_SM))
                    .on_press(TemplatesMessage::New)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Primary.style()),
            )
            .into();
    }
    list = list.push(hint(t!("main.side_panel.templates.insert_hint")));

    let mut shown = 0_usize;
    for template in state
        .templates
        .iter()
        .filter(|template| template.matches(&state.query))
    {
        shown += 1;
        list = list.push(
            if state.confirm_delete.as_deref() == Some(template.id.as_str()) {
                confirm_row(template)
            } else {
                template_row(template)
            },
        );
    }
    if shown == 0 {
        list = list.push(hint(t!(
            "main.side_panel.templates.no_match",
            query = state.query.trim()
        )));
    }
    list.into()
}

fn hint<'a>(message: std::borrow::Cow<'a, str>) -> Element<'a, TemplatesMessage> {
    text(message)
        .size(TEXT_SM)
        .style(theme::TextClass::Tertiary.style())
        .into()
}

fn template_row(template: &Template) -> Element<'_, TemplatesMessage> {
    let detail = template
        .subject
        .as_deref()
        .filter(|subject| !subject.is_empty())
        .unwrap_or(&template.preview);
    let summary = column![
        text(&template.name)
            .size(TEXT_MD)
            .font(crate::font::text_semibold())
            .wrapping(text::Wrapping::None),
        text(detail)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
            .wrapping(text::Wrapping::None),
    ]
    .spacing(SPACE_XXXS);
    let insert = button(container(summary).width(Length::Fill).clip(true))
        .on_press(TemplatesMessage::Insert(template.id.clone()))
        .padding(PAD_NAV_ITEM)
        .style(theme::ButtonClass::Action.style())
        .width(Length::Fill);
    row![
        insert,
        widgets::nav_icon_button(
            icon::pencil(),
            false,
            TemplatesMessage::Edit(template.id.clone()),
        ),
        widgets::nav_icon_button(
            icon::trash(),
            false,
            TemplatesMessage::Delete(template.id.clone()),
        ),
    ]
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center)
    .into()
}

fn confirm_row(template: &Template) -> Element<'_, TemplatesMessage> {
    let question = text(t!(
        "main.side_panel.templates.delete_confirm",
        name = template.name.as_str()
    ))
    .size(TEXT_SM)
    .style(text::danger);
    let actions = row![
        Space::new().width(Length::Fill),
        text_button(
            t!("main.side_panel.templates.cancel"),
            TemplatesMessage::CancelDelete
        ),
        button(text(t!("main.side_panel.templates.delete")).size(TEXT_SM))
            .on_press(TemplatesMessage::ConfirmDelete(template.id.clone()))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Destructive.style()),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    container(column![question, actions].spacing(SPACE_XS))
        .padding(PAD_NAV_ITEM)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

fn text_button<'a>(
    label: std::borrow::Cow<'a, str>,
    on_press: TemplatesMessage,
) -> Element<'a, TemplatesMessage> {
    button(text(label).size(TEXT_SM).style(text::base))
        .on_press(on_press)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Secondary.style())
        .into()
}

fn editor_view(editor: &Editor) -> Element<'_, TemplatesMessage> {
    let title = if editor.id.is_some() {
        t!("main.side_panel.templates.edit_title")
    } else {
        t!("main.side_panel.templates.new_title")
    };
    let name = text_input(
        t!("main.side_panel.templates.name_placeholder"),
        &editor.name,
    )
    .on_input(TemplatesMessage::Name)
    .padding(PAD_INPUT)
    .size(TEXT_SM);
    let subject = text_input(
        t!("main.side_panel.templates.subject_placeholder"),
        &editor.subject,
    )
    .on_input(TemplatesMessage::Subject)
    .padding(PAD_INPUT)
    .size(TEXT_SM);
    let body = container(
        rich_text_editor(&editor.body)
            .on_action(TemplatesMessage::Body)
            .font(crate::font::text())
            .height(Length::Fixed(TEMPLATE_EDITOR_HEIGHT))
            .width(Length::Fill),
    )
    .padding(PAD_NAV_ITEM)
    .style(theme::ContainerClass::EditorField.style());

    let mut save = button(text(t!("main.side_panel.templates.save")).size(TEXT_SM))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style());
    if editor.can_save() {
        save = save.on_press(TemplatesMessage::Save);
    }
    let actions = row![
        Space::new().width(Length::Fill),
        text_button(
            t!("main.side_panel.templates.cancel"),
            TemplatesMessage::Cancel
        ),
        save,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    column![
        text(title).size(TEXT_XL).font(crate::font::text_semibold()),
        field(t!("main.side_panel.templates.name"), name.into()),
        field(t!("main.side_panel.templates.subject"), subject.into()),
        field(
            t!("main.side_panel.templates.body"),
            column![formatting_bar(editor), body]
                .spacing(SPACE_XXS)
                .into()
        ),
        actions,
    ]
    .spacing(SPACE_SM)
    .into()
}

fn field<'a>(
    label: std::borrow::Cow<'a, str>,
    input: Element<'a, TemplatesMessage>,
) -> Element<'a, TemplatesMessage> {
    column![
        text(label)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        input,
    ]
    .spacing(SPACE_XXS)
    .into()
}

fn formatting_bar<'a>(editor: &'a Editor) -> Element<'a, TemplatesMessage> {
    let edit = |action| TemplatesMessage::Body(RteAction::Edit(action));
    let style_tool = |glyph: fn() -> iced::widget::Text<'a>, label, style| {
        widgets::Tool::icon(glyph, label, edit(EditAction::ToggleInlineStyle(style)))
    };
    let list_tool = |glyph: fn() -> iced::widget::Text<'a>, label, ordered| {
        widgets::Tool::icon(
            glyph,
            label,
            edit(EditAction::SetBlockType(BlockKind::ListItem { ordered })),
        )
    };
    let bar = widgets::format_bar(
        &editor.format_bar,
        widgets::CaretContext::of(&editor.body),
        TemplatesMessage::ToggleFormatMenu,
        TemplatesMessage::FormatQuery,
        edit,
    );
    // The side panel is narrow: the last tools fold into the "more" panel
    // so the bar holds on one line.
    let tools = vec![
        bar.font,
        bar.size,
        style_tool(icon::bold, t!("compose.format.bold"), InlineStyle::BOLD),
        style_tool(
            icon::italic,
            t!("compose.format.italic"),
            InlineStyle::ITALIC,
        ),
        style_tool(
            icon::underline,
            t!("compose.format.underline"),
            InlineStyle::UNDERLINE,
        ),
        bar.color,
        bar.highlight,
        list_tool(icon::list, t!("compose.format.bulleted_list"), false),
        list_tool(icon::list_ordered, t!("compose.format.numbered_list"), true),
        bar.alignment,
        bar.block,
        style_tool(
            icon::strikethrough,
            t!("compose.format.strikethrough"),
            InlineStyle::STRIKETHROUGH,
        ),
        bar.line_height,
        bar.outdent,
        bar.indent,
        bar.smaller,
        bar.larger,
        bar.table,
        bar.clear,
    ];
    widgets::toolbar(
        tools,
        &editor.format_bar,
        TemplatesMessage::ToggleFormatMenu,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str, subject: Option<&str>, body: &str) -> Template {
        Template::new(
            "id".to_string(),
            name.to_string(),
            subject.map(str::to_string),
            body.to_string(),
        )
    }

    #[test]
    fn preview_joins_the_body_lines() {
        let reply = template("Relance", None, "<p>Bonjour,</p><p></p><p>Merci</p>");
        assert_eq!(reply.preview, "Bonjour, Merci");
    }

    #[test]
    fn search_looks_at_name_subject_and_text_ignoring_case() {
        let quote = template("Devis", Some("Votre devis"), "<p>Ci-joint le tarif</p>");
        assert!(quote.matches(""));
        assert!(quote.matches("dev"));
        assert!(quote.matches("VOTRE"));
        assert!(quote.matches("tarif"));
        assert!(!quote.matches("facture"));
    }

    #[test]
    fn editing_starts_from_the_stored_template() {
        let mut state = Templates {
            templates: vec![template("Devis", Some("Votre devis"), "<p>Bonjour</p>")],
            ..Templates::default()
        };
        state.start_edit("id");
        let editor = state.editor.as_ref().expect("editor open");
        assert_eq!(editor.id.as_deref(), Some("id"));
        assert_eq!(editor.subject, "Votre devis");
        assert!(editor.body.to_html().contains("Bonjour"));
        assert!(editor.can_save());
    }
}
