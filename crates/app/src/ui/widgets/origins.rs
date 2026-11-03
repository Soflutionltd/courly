//! Marks naming the address books a contact comes from: the apple for the
//! Mac's address book, the Microsoft squares for Outlook, and so on.

use std::sync::LazyLock;

use iced::widget::{Row, svg, text};
use iced::{Alignment, Element};
use rtsk::contacts::search::ContactOrigins;

use crate::icon;
use crate::ui::layout::SPACE_XXS;

static APPLE: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../../../../assets/brands/apple.svg"))
});
static MICROSOFT: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../../../../assets/brands/microsoft.svg"))
});
static GOOGLE: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../../../../assets/brands/google.svg"))
});
static COURLY: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../../../../../assets/icon.svg")));

/// The marks of every address book holding the contact, or `None` for an
/// address only seen in mail.
pub fn contact_origin_marks<'a, M: 'a>(
    origins: ContactOrigins,
    size: f32,
) -> Option<Element<'a, M>> {
    let mut marks: Vec<Element<'a, M>> = Vec::new();
    if origins.apple {
        marks.push(monochrome(&APPLE, size));
    }
    if origins.microsoft {
        marks.push(in_color(&MICROSOFT, size));
    }
    if origins.google {
        marks.push(monochrome(&GOOGLE, size));
    }
    if origins.other_book {
        marks.push(icon::user().size(size).style(text::secondary).into());
    }
    if origins.courly {
        marks.push(in_color(&COURLY, size));
    }
    (!marks.is_empty()).then(|| {
        Row::with_children(marks)
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center)
            .into()
    })
}

fn in_color<'a, M: 'a>(handle: &svg::Handle, size: f32) -> Element<'a, M> {
    svg(handle.clone()).width(size).height(size).into()
}

/// Brand marks drawn in one color take the text color, so they read on the
/// light and the dark theme alike.
fn monochrome<'a, M: 'a>(handle: &svg::Handle, size: f32) -> Element<'a, M> {
    svg(handle.clone())
        .width(size)
        .height(size)
        .style(|theme: &iced::Theme, _status| svg::Style {
            color: Some(theme.palette().background.base.text),
        })
        .into()
}
