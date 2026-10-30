//! The panel right of the mail view. It shows one module at a time and
//! opens or closes only when the user asks: no window size folds it.

pub mod my_day;
pub mod templates;

use iced::widget::{Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length};
use jiff::civil::Weekday;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::Message;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

/// A module the panel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Module {
    #[default]
    MyDay,
    Templates,
}

impl Module {
    pub const ALL: [Self; 2] = [Self::MyDay, Self::Templates];

    fn icon<'a>(self) -> iced::widget::Text<'a> {
        match self {
            Self::MyDay => icon::calendar(),
            Self::Templates => icon::layout_template(),
        }
    }

    fn name(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::MyDay => t!("main.side_panel.module.my_day"),
            Self::Templates => t!("main.side_panel.module.templates"),
        }
    }
}

pub struct SidePanel {
    pub open: bool,
    pub module: Module,
    pub my_day: my_day::MyDay,
    pub templates: templates::Templates,
}

impl SidePanel {
    pub fn new(open: bool, module: Module) -> Self {
        Self {
            open,
            module,
            my_day: my_day::MyDay::new(jiff::Zoned::now().date()),
            templates: templates::Templates::default(),
        }
    }

    /// Whether the My Day agenda is on screen and needs fresh events.
    pub fn shows_my_day(&self) -> bool {
        self.open && self.module == Module::MyDay
    }

    /// Whether the templates list is on screen and needs loading.
    pub fn shows_templates(&self) -> bool {
        self.open && self.module == Module::Templates
    }
}

#[derive(Debug, Clone)]
pub enum SidePanelMessage {
    Toggle,
    Close,
    Show(Module),
    MyDay(my_day::MyDayMessage),
    Templates(templates::TemplatesMessage),
}

fn panel_message(message: SidePanelMessage) -> Message {
    Message::SidePanel(message)
}

pub fn view(panel: &SidePanel, week_start: Weekday) -> Element<'_, Message> {
    let mut tabs = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    for module in Module::ALL {
        tabs = tabs.push(widgets::tip_icon_button(
            module.icon(),
            module.name(),
            module == panel.module,
            panel_message(SidePanelMessage::Show(module)),
        ));
    }
    let header = container(
        row![
            tabs,
            Space::new().width(Length::Fill),
            widgets::tip_icon_button_at(
                icon::panel_right_close(),
                t!("main.side_panel.close"),
                false,
                panel_message(SidePanelMessage::Close),
                iced::widget::tooltip::Position::Left,
            ),
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_SIDE_PANEL);

    let body = match panel.module {
        Module::MyDay => my_day::view(&panel.my_day, week_start)
            .map(|message| panel_message(SidePanelMessage::MyDay(message))),
        Module::Templates => templates::view(&panel.templates)
            .map(|message| panel_message(SidePanelMessage::Templates(message))),
    };

    container(column![
        header,
        widgets::divider(),
        scrollable(body)
            .spacing(SCROLLBAR_SPACING)
            .height(Length::Fill),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .clip(true)
    .style(theme::ContainerClass::Pane.style())
    .into()
}
