use iced::advanced::Renderer as _;
use iced::advanced::{Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

const POINT_ANCHORED_POPUP_MIN_WIDTH: f32 = 160.0;

/// A widget that displays a base element and optionally floats a popup
/// overlay relative to it, without affecting layout. Clicks outside the
/// popup dismiss it via `on_dismiss`.
pub struct AnchoredOverlay<'a, Message> {
    base: iced::Element<'a, Message>,
    popup: Option<iced::Element<'a, Message>>,
    on_dismiss: Option<Message>,
    popup_width: Option<f32>,
    position: AnchorPosition,
    anchor_point: Option<Point>,
    anchor_offset: Option<Vector>,
    on_right_press: Option<Box<dyn Fn(Point) -> Message + 'a>>,
    on_right_press_offset: Option<Box<dyn Fn(Vector) -> Message + 'a>>,
}

/// Where the popup appears relative to the base widget.
#[derive(Debug, Clone, Copy, Default)]
pub enum AnchorPosition {
    /// Below the base, left-aligned.
    #[default]
    Below,
    /// Below the base, right edge aligned with the base's right edge.
    BelowRight,
}

pub fn anchored_overlay<'a, Message: 'a>(
    base: impl Into<iced::Element<'a, Message>>,
) -> AnchoredOverlay<'a, Message> {
    AnchoredOverlay {
        base: base.into(),
        popup: None,
        on_dismiss: None,
        popup_width: None,
        position: AnchorPosition::default(),
        anchor_point: None,
        anchor_offset: None,
        on_right_press: None,
        on_right_press_offset: None,
    }
}

impl<'a, Message: Clone + 'a> AnchoredOverlay<'a, Message> {
    pub fn popup(mut self, popup: impl Into<iced::Element<'a, Message>>) -> Self {
        self.popup = Some(popup.into());
        self
    }

    pub fn on_dismiss(mut self, message: Message) -> Self {
        self.on_dismiss = Some(message);
        self
    }

    /// Set a fixed width for the popup. If unset, the popup uses the base's
    /// width, or a small safe default when point-anchored.
    pub fn popup_width(mut self, width: f32) -> Self {
        self.popup_width = Some(width);
        self
    }

    pub fn position(mut self, position: AnchorPosition) -> Self {
        self.position = position;
        self
    }

    pub fn anchor_point(mut self, point: Point) -> Self {
        self.anchor_point = Some(point);
        self
    }

    /// Report a right-click on the base that no child handled, with the
    /// cursor's window position (for a context menu at that point).
    pub fn on_right_press(mut self, on_right_press: impl Fn(Point) -> Message + 'a) -> Self {
        self.on_right_press = Some(Box::new(on_right_press));
        self
    }

    /// Float the popup at `offset` from the base's top-left corner. Unlike
    /// [`Self::anchor_point`], the popup follows the base when it scrolls.
    pub fn anchor_offset(mut self, offset: Vector) -> Self {
        self.anchor_offset = Some(offset);
        self
    }

    /// Like [`Self::on_right_press`], with the click measured from the
    /// base's top-left corner, for [`Self::anchor_offset`]. Works inside a
    /// scrollable, where window coordinates go stale as the content moves.
    pub fn on_right_press_offset(
        mut self,
        on_right_press: impl Fn(Vector) -> Message + 'a,
    ) -> Self {
        self.on_right_press_offset = Some(Box::new(on_right_press));
        self
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for AnchoredOverlay<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.base.as_widget().size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.base
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.base.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        match &mut self.popup {
            Some(popup) => {
                tree.diff_children(&mut [self.base.as_widget_mut(), popup.as_widget_mut()]);
            }
            None => tree.diff_children(std::slice::from_mut(&mut self.base)),
        }
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation<()>,
    ) {
        self.base.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            viewport,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.base.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );

        if let Some(on_right_press) = self.on_right_press.as_ref()
            && !shell.is_event_captured()
            && let Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Right,
                ..
            }) = event
            && let Some(position) = cursor.position_over(layout.bounds())
        {
            shell.publish(on_right_press(position));
            shell.capture_event();
        }

        if let Some(on_right_press) = self.on_right_press_offset.as_ref()
            && !shell.is_event_captured()
            && let Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Right,
                ..
            }) = event
            && let Some(position) = cursor.position_over(layout.bounds())
        {
            shell.publish(on_right_press(position - layout.position()));
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.base.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let Some(popup) = self.popup.as_mut() else {
            return self.base.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
                window,
            );
        };

        let (first, second) = tree.children.split_at_mut(1);

        let mut overlays = self.base.as_widget_mut().overlay(
            &mut first[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        );

        let popup_tree = &mut second[0];
        let base_position = layout.position() + translation;
        let anchor_point = self
            .anchor_offset
            .map(|offset| base_position + offset)
            .or(self.anchor_point);
        let popup_layout = layout_popup(
            popup,
            popup_tree,
            renderer,
            window,
            anchor_point.unwrap_or(base_position),
            if anchor_point.is_some() {
                Size::ZERO
            } else {
                layout.bounds().size()
            },
            self.popup_width,
            anchor_point.is_some(),
            self.position,
        );

        overlays.push(overlay::Element::new(Box::new(AnchoredOverlayLayer {
            layout: popup_layout,
            content: popup,
            tree: popup_tree,
            viewport: *viewport,
            window,
            on_dismiss: self.on_dismiss.clone(),
        })));

        overlays
    }
}

/// Lay the popup out in its own subtree and place it under the anchor (or
/// above it when only that side has room), clamped horizontally so it stays
/// inside the window.
#[allow(clippy::too_many_arguments)]
fn layout_popup<Message>(
    popup: &mut iced::Element<'_, Message>,
    tree: &mut widget::Tree,
    renderer: &Renderer,
    window: Size,
    anchor_position: Point,
    anchor_size: Size,
    popup_width: Option<f32>,
    point_anchored: bool,
    position: AnchorPosition,
) -> Layout {
    let popup_width = popup_width.unwrap_or(if point_anchored {
        POINT_ANCHORED_POPUP_MIN_WIDTH
    } else {
        anchor_size.width
    });
    let below_y = anchor_position.y + anchor_size.height;
    let space_below = (window.height - below_y).max(0.0);
    let space_above = anchor_position.y.max(0.0);

    // A context menu may use the whole window height, like a native one.
    let max_height = if point_anchored {
        window.height
    } else {
        space_below.max(space_above)
    };
    let limits = layout::Limits::new(
        Size::ZERO,
        Size {
            width: popup_width,
            height: max_height,
        },
    )
    .width(Length::Fill);

    popup.as_widget_mut().layout(tree, renderer, &limits);
    let size = tree.size;

    let y = if size.height <= space_below {
        below_y
    } else if point_anchored && size.height > space_above {
        // Fits on neither side of the click: slide it up just enough to
        // end at the bottom of the window.
        (window.height - size.height).max(0.0)
    } else if space_above > space_below {
        // Open upward when the popup does not fit below and the room above is larger.
        (anchor_position.y - size.height).max(0.0)
    } else {
        below_y
    };

    // Calculate X based on position mode
    let x = match position {
        AnchorPosition::Below => anchor_position.x,
        AnchorPosition::BelowRight => {
            let right_edge = anchor_position.x + anchor_size.width;
            (right_edge - size.width).max(0.0)
        }
    };

    // Clamp so popup stays within viewport
    let x = x.clamp(0.0, (window.width - size.width).max(0.0));

    Layout::new(size).move_to(Point::new(x, y))
}

impl<'a, Message: Clone + 'a> From<AnchoredOverlay<'a, Message>> for iced::Element<'a, Message> {
    fn from(overlay: AnchoredOverlay<'a, Message>) -> Self {
        iced::Element::new(overlay)
    }
}

struct AnchoredOverlayLayer<'a, 'b, Message> {
    layout: Layout,
    content: &'b mut iced::Element<'a, Message>,
    tree: &'b mut widget::Tree,
    viewport: Rectangle,
    window: Size,
    on_dismiss: Option<Message>,
}

impl<Message: Clone> overlay::Overlay<Message, Theme, Renderer>
    for AnchoredOverlayLayer<'_, '_, Message>
{
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        let layout = self.layout;
        renderer.with_layer(Rectangle::with_size(self.window), |renderer| {
            self.content.as_widget().draw(
                self.tree,
                renderer,
                theme,
                style,
                layout,
                cursor,
                &layout.bounds(),
            );
        });
    }

    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn widget::Operation<()>) {
        let layout = self.layout;
        self.content.as_widget_mut().operate(
            self.tree,
            layout,
            &layout.bounds(),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let layout = self.layout;
        self.content.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            shell,
            &layout.bounds(),
        );

        // Clicks inside the overlay are captured so they don't propagate.
        // Clicks outside the overlay dismiss it.
        if let Event::Mouse(mouse::Event::ButtonPressed {
            button: mouse::Button::Left,
            ..
        })
        | Event::Touch(iced::touch::Event::FingerPressed { .. }) = event
        {
            if cursor.is_over(layout.bounds()) {
                shell.capture_event();
            } else if let Some(on_dismiss) = &self.on_dismiss {
                shell.publish(on_dismiss.clone());
                shell.capture_event();
            }
        } else if matches!(event, Event::Mouse(_) | Event::Touch(_))
            && cursor.is_over(layout.bounds())
        {
            shell.capture_event();
        }
    }

    fn mouse_interaction(&self, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        let layout = self.layout;
        let interaction = self.content.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        );

        // If cursor is over the overlay but between child widgets (e.g. in
        // spacing gaps), the content returns Interaction::None.  iced treats
        // None as "not interactive" and passes the cursor through to base
        // widgets underneath, causing hover states to bleed through.  Return
        // Idle instead so iced blocks the cursor from the base layer.
        if interaction == mouse::Interaction::None && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Idle
        } else {
            interaction
        }
    }

    fn overlay<'c>(
        &'c mut self,
        renderer: &Renderer,
    ) -> Vec<overlay::Element<'c, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            self.tree,
            self.layout,
            renderer,
            &self.viewport,
            Vector::default(),
            self.window,
        )
    }
}
