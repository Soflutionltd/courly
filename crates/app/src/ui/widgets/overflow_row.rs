//! A row of tools that always holds on one line: the tools that do not fit
//! are hidden from the end and a "more" button takes their place, the way
//! Outlook's formatting ribbon folds into its ellipsis.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use iced::advanced::{Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};

/// How many of the tools of widths `widths` hold in `room` with `spacing`
/// between them: all of them when they fit as they are, otherwise as many as
/// fit followed by a "more" button of width `more`.
pub(crate) fn tools_that_fit(widths: &[f32], more: f32, spacing: f32, room: f32) -> usize {
    let all = widths.iter().enumerate().fold(0.0, |used, (index, width)| {
        used + width + if index == 0 { 0.0 } else { spacing }
    });
    if all <= room {
        return widths.len();
    }
    let mut used = more;
    for (index, width) in widths.iter().enumerate() {
        let next = used + width + spacing;
        if next > room {
            return index;
        }
        used = next;
    }
    widths.len()
}

/// See the module documentation. `shown` receives the number of tools left
/// in the row at every layout, for the host to fill its overflow panel with
/// the others.
pub struct OverflowRow<'a, M> {
    /// The tools, then the "more" button.
    children: Vec<Element<'a, M>>,
    spacing: f32,
    shown: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy, Default)]
struct State {
    /// Tools laid out in the row.
    shown: usize,
    /// Whether the "more" button is in the row.
    more: bool,
}

pub fn overflow_row<'a, M>(
    tools: Vec<Element<'a, M>>,
    more: Element<'a, M>,
    spacing: f32,
    shown: Arc<AtomicUsize>,
) -> OverflowRow<'a, M> {
    let mut children = tools;
    children.push(more);
    OverflowRow {
        children,
        spacing,
        shown,
    }
}

impl<M> OverflowRow<'_, M> {
    fn tool_count(&self) -> usize {
        self.children.len() - 1
    }
}

impl State {
    fn is_visible(&self, index: usize, tool_count: usize) -> bool {
        index < self.shown || (index == tool_count && self.more)
    }
}

impl<M> Widget<M, Theme, Renderer> for OverflowRow<'_, M> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(&mut self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) {
        let limits = limits.width(Length::Fill).height(Length::Shrink);
        let bounds = limits.bounds();
        let room = bounds.width;
        let child_limits = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, bounds.height));

        let mut widths = Vec::with_capacity(self.children.len());
        for (child, child_tree) in self.children.iter_mut().zip(tree.children.iter_mut()) {
            child
                .as_widget_mut()
                .layout(child_tree, renderer, &child_limits);
            widths.push(child_tree.size.width);
        }
        let tool_count = self.tool_count();
        let more_width = widths[tool_count];
        let shown = tools_that_fit(&widths[..tool_count], more_width, self.spacing, room);
        let state = State {
            shown,
            more: shown < tool_count,
        };

        let height = tree
            .children
            .iter()
            .enumerate()
            .filter(|(index, _)| state.is_visible(*index, tool_count))
            .fold(0.0_f32, |tallest, (_, child)| {
                tallest.max(child.size.height)
            });
        let mut x = 0.0;
        for (index, child) in tree.children.iter_mut().enumerate() {
            if state.is_visible(index, tool_count) {
                child.translation = Vector::new(x, (height - child.size.height) / 2.0);
                x += child.size.width + self.spacing;
            } else {
                child.translation = Vector::ZERO;
                child.size = Size::ZERO;
            }
        }
        let used = (x - self.spacing).max(0.0);

        self.shown.store(shown, Ordering::Relaxed);
        *tree.state.downcast_mut::<State>() = state;
        tree.size = limits.resolve(Length::Fill, Length::Shrink, Size::new(used, height));
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation<()>,
    ) {
        let tool_count = self.tool_count();
        let state = *tree.state.downcast_ref::<State>();
        operation.container(None, layout.bounds(), viewport);
        operation.traverse(&mut |operation| {
            for (index, (child, (child_layout, child_tree))) in self
                .children
                .iter_mut()
                .zip(layout.iter_mut(&mut tree.children))
                .enumerate()
            {
                if state.is_visible(index, tool_count) {
                    child.as_widget_mut().operate(
                        child_tree,
                        child_layout,
                        viewport,
                        renderer,
                        operation,
                    );
                }
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let tool_count = self.tool_count();
        let state = *tree.state.downcast_ref::<State>();
        for (index, (child, (child_layout, child_tree))) in self
            .children
            .iter_mut()
            .zip(layout.iter_mut(&mut tree.children))
            .enumerate()
        {
            if state.is_visible(index, tool_count) {
                child.as_widget_mut().update(
                    child_tree,
                    event,
                    child_layout,
                    cursor,
                    renderer,
                    shell,
                    viewport,
                );
            }
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
        let tool_count = self.tool_count();
        let state = tree.state.downcast_ref::<State>();
        self.children
            .iter()
            .zip(layout.iter(&tree.children))
            .enumerate()
            .filter(|(index, _)| state.is_visible(*index, tool_count))
            .map(|(_, (child, (child_layout, child_tree)))| {
                child.as_widget().mouse_interaction(
                    child_tree,
                    child_layout,
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .max()
            .unwrap_or_default()
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
        let tool_count = self.tool_count();
        let state = tree.state.downcast_ref::<State>();
        for (index, (child, (child_layout, child_tree))) in self
            .children
            .iter()
            .zip(layout.iter(&tree.children))
            .enumerate()
        {
            if state.is_visible(index, tool_count) {
                child.as_widget().draw(
                    child_tree,
                    renderer,
                    theme,
                    style,
                    child_layout,
                    cursor,
                    viewport,
                );
            }
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, M, Theme, Renderer>> {
        let tool_count = self.tool_count();
        let state = *tree.state.downcast_ref::<State>();
        self.children
            .iter_mut()
            .zip(layout.iter_mut(&mut tree.children))
            .enumerate()
            .filter(|(index, _)| state.is_visible(*index, tool_count))
            .flat_map(|(_, (child, (child_layout, child_tree)))| {
                child.as_widget_mut().overlay(
                    child_tree,
                    child_layout,
                    renderer,
                    viewport,
                    translation,
                    window,
                )
            })
            .collect()
    }
}

impl<'a, M: 'a> From<OverflowRow<'a, M>> for Element<'a, M> {
    fn from(row: OverflowRow<'a, M>) -> Self {
        Element::new(row)
    }
}

#[cfg(test)]
mod tests {
    use super::tools_that_fit;

    #[test]
    fn every_tool_stays_when_they_all_fit_without_the_more_button() {
        assert_eq!(tools_that_fit(&[30.0, 30.0, 30.0], 30.0, 4.0, 98.0), 3);
    }

    #[test]
    fn the_tools_that_do_not_fit_leave_room_for_the_more_button() {
        // 30 + 4 + 30 + 4 + more 30 = 98 holds; a third tool would not.
        assert_eq!(
            tools_that_fit(&[30.0, 30.0, 30.0, 30.0], 30.0, 4.0, 100.0),
            2
        );
    }

    #[test]
    fn a_bar_narrower_than_the_more_button_keeps_no_tool() {
        assert_eq!(tools_that_fit(&[30.0, 30.0], 30.0, 4.0, 20.0), 0);
    }

    #[test]
    fn no_tool_needs_no_room() {
        assert_eq!(tools_that_fit(&[], 30.0, 4.0, 0.0), 0);
    }
}
