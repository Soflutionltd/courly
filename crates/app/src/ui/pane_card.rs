//! The rounded card a main-window pane floats in.
//!
//! iced clips children to a rectangle, so a child that paints its own
//! background fills the card's corners square. A frame drawn on top of the
//! content repaints what sticks out of the rounding with the canvas colour,
//! then draws the border.

use iced::advanced::layout;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::widget::Tree;
use iced::advanced::{Layout, Widget};
use iced::widget::{container, stack};
use iced::{Border, Color, Element, Length, Rectangle, Size, Theme, mouse};

use crate::ui::layout::RADIUS_LG;
use crate::ui::theme::{self, ContainerClass};

/// Wraps `content` in a rounded pane card.
pub fn pane_card<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    stack![
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .clip(true)
            .style(ContainerClass::PaneBody.style()),
        Element::new(CornerFrame),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Draws over the content only; it takes no input, so the content under it
/// keeps the cursor.
struct CornerFrame;

impl<Message> Widget<Message, Theme, iced::Renderer> for CornerFrame {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) {
        tree.size = layout::atomic(limits, Length::Fill, Length::Fill);
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let (canvas, border) = theme::pane_frame_colors(theme);
        // A quad's border is its outline eroded by the border width, so a
        // quad grown by the radius with a doubled radius has the card's
        // rounded outline as its inner edge. Clipped to the card, only the
        // corners outside the rounding get painted.
        renderer.with_layer(bounds, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bounds.expand(RADIUS_LG),
                    border: Border {
                        color: canvas,
                        width: RADIUS_LG,
                        radius: (2.0 * RADIUS_LG).into(),
                    },
                    ..renderer::Quad::default()
                },
                Color::TRANSPARENT,
            );
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: Border {
                        color: border,
                        width: 1.0,
                        radius: RADIUS_LG.into(),
                    },
                    ..renderer::Quad::default()
                },
                Color::TRANSPARENT,
            );
        });
    }
}
