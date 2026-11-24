//! Scroll a list so that one of its children sits at the top.
//!
//! The offset of a child is only known to the layout, so the operation reads
//! it there: it walks the tree once for the top of the list's content and of
//! the child, then chains the scroll.

use iced::Rectangle;
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, scroll_to};
use iced::advanced::widget::operation::{Animation, Outcome};
use iced::advanced::widget::{Id, Operation};

/// Scroll the scrollable `list` so that the container `child`, somewhere in
/// its content, is at the top of the view, `below` pixels down into it.
/// Does nothing when either is not in the tree.
pub fn scroll_to_child<T: 'static>(list: Id, child: Id, below: f32) -> impl Operation<T> {
    ScrollToChild {
        list,
        child,
        below,
        content_top: None,
        child_top: None,
    }
}

struct ScrollToChild {
    list: Id,
    child: Id,
    below: f32,
    content_top: Option<f32>,
    child_top: Option<f32>,
}

impl<T: 'static> Operation<T> for ScrollToChild {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        // A scrollable reports its content as a container under its own id.
        // Layout bounds ignore the current scroll position, so the distance
        // between the two tops is the offset to scroll to.
        if id == Some(&self.list) {
            self.content_top.get_or_insert(bounds.y);
        } else if id == Some(&self.child) {
            self.child_top.get_or_insert(bounds.y);
        }
    }

    fn finish(&self) -> Outcome<T> {
        match (self.content_top, self.child_top) {
            (Some(content_top), Some(child_top)) => Outcome::Chain(Box::new(scroll_to(
                self.list.clone(),
                AbsoluteOffset {
                    x: None,
                    y: Some((child_top - content_top + self.below).max(0.0)),
                },
                Animation::Instant,
            ))),
            _ => Outcome::None,
        }
    }
}
