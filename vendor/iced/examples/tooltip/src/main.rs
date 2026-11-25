use iced::Element;
use iced::alignment;
use iced::time::seconds;
use iced::widget::tooltip::Position;
use iced::widget::{button, center, checkbox, column, container, text, tooltip};

pub fn main() -> iced::Result {
    iced::run(Tooltip::update, Tooltip::view)
}

#[derive(Default)]
struct Tooltip {
    position: Position,
    delay: bool,
}

#[derive(Debug, Clone)]
enum Message {
    ChangePosition,
    ToggleDelay(bool),
}

impl Tooltip {
    fn update(&mut self, message: Message) {
        match message {
            Message::ChangePosition => {
                let position = match &self.position {
                    Position::Top => Position::TopRight,
                    Position::TopRight => Position::RightTop,
                    Position::RightTop => Position::Right,
                    Position::Right => Position::RightBottom,
                    Position::RightBottom => Position::BottomRight,
                    Position::BottomRight => Position::Bottom,
                    Position::Bottom => Position::BottomLeft,
                    Position::BottomLeft => Position::LeftBottom,
                    Position::LeftBottom => Position::Left,
                    Position::Left => Position::LeftTop,
                    Position::LeftTop => Position::FollowCursor,
                    Position::FollowCursor => Position::TopLeft,
                    Position::TopLeft => Position::Top,
                };

                self.position = position;
            }
            Message::ToggleDelay(is_immediate) => {
                self.delay = is_immediate;
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let tooltip = tooltip(
            button(text("Press\nto\nchange position").align_x(alignment::Horizontal::Center))
                .on_press(Message::ChangePosition),
            position_to_text(self.position),
            self.position,
        )
        .gap(10)
        .delay(seconds(if self.delay { 1 } else { 0 }))
        .style(container::rounded_box);

        let checkbox = checkbox(self.delay)
            .label("Delay")
            .on_toggle(Message::ToggleDelay);

        center(
            column![tooltip, checkbox]
                .align_x(alignment::Horizontal::Center)
                .spacing(10),
        )
        .into()
    }
}

fn position_to_text<'a>(position: Position) -> &'a str {
    match position {
        Position::FollowCursor => "Follow Cursor",
        Position::TopLeft => "Top Left",
        Position::Top => "Top",
        Position::TopRight => "Top Right",
        Position::RightTop => "Right Top",
        Position::Right => "Right",
        Position::RightBottom => "Right Bottom",
        Position::BottomLeft => "Bottom Left",
        Position::Bottom => "Bottom",
        Position::BottomRight => "Bottom Right",
        Position::LeftTop => "Left Top",
        Position::Left => "Left",
        Position::LeftBottom => "Left Bottom",
    }
}
