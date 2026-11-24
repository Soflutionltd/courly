//! Read how far down the message list and the open conversation are.
//!
//! The position of a scrollable lives in the widget tree, not in the state
//! of the app. Following it with a message per scroll event would rebuild
//! the view on every wheel tick, so it is read once, when it is needed.

use iced::advanced::widget::operation::{Outcome, Scrollable};
use iced::advanced::widget::{Id, Operation};
use iced::{Rectangle, Size, Vector};

/// What the view reported. `None` for a list that is not on screen.
#[derive(Debug, Clone, Default)]
pub struct ViewPlace {
    pub list_offset: Option<f32>,
    pub reading_offset: Option<f32>,
    /// The top of each message card of the conversation, from the top of
    /// its content, `None` for a card not in the tree.
    pub card_tops: Vec<Option<f32>>,
}

impl ViewPlace {
    /// The message at the top of the conversation view and how far down
    /// into it the view is. Heights above it may change from one launch to
    /// the next (a mail drawn a little differently): a place held this way
    /// survives that, where a bare offset would drift.
    pub fn reading_anchor(&self) -> Option<(usize, f32)> {
        let offset = self.reading_offset?;
        self.card_tops
            .iter()
            .enumerate()
            .filter_map(|(index, top)| top.map(|top| (index, top)))
            .take_while(|(_, top)| *top <= offset)
            .last()
            .map(|(index, top)| (index, offset - top))
    }
}

/// Read the offsets of the scrollables `list` and `reading`, and the tops
/// of the `cards` laid out in `reading`.
pub fn view_place(list: Id, reading: Id, cards: Vec<Id>) -> impl Operation<ViewPlace> {
    Probe {
        list,
        reading,
        card_tops: vec![None; cards.len()],
        cards,
        reading_top: None,
        found: ViewPlace::default(),
    }
}

struct Probe {
    list: Id,
    reading: Id,
    cards: Vec<Id>,
    card_tops: Vec<Option<f32>>,
    reading_top: Option<f32>,
    found: ViewPlace,
}

impl Operation<ViewPlace> for Probe {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<ViewPlace>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        // A scrollable reports its content as a container under its own id,
        // laid out without the scroll: tops are measured from there.
        let Some(id) = id else {
            return;
        };
        if *id == self.reading {
            self.reading_top.get_or_insert(bounds.y);
        } else if let Some(index) = self.cards.iter().position(|card| card == id) {
            self.card_tops[index].get_or_insert(bounds.y);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content: Size,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.list) {
            self.found.list_offset = Some(translation.y);
        } else if id == Some(&self.reading) {
            self.found.reading_offset = Some(translation.y);
        }
    }

    fn finish(&self) -> Outcome<ViewPlace> {
        let mut found = self.found.clone();
        found.card_tops = match self.reading_top {
            Some(top) => self
                .card_tops
                .iter()
                .map(|card| card.map(|card| card - top))
                .collect(),
            None => Vec::new(),
        };
        Outcome::Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::ViewPlace;

    fn place(offset: f32, tops: &[f32]) -> ViewPlace {
        ViewPlace {
            list_offset: None,
            reading_offset: Some(offset),
            card_tops: tops.iter().copied().map(Some).collect(),
        }
    }

    #[test]
    fn the_anchor_is_the_card_at_the_top_and_the_distance_into_it() {
        assert_eq!(
            place(800.0, &[0.0, 300.0, 740.0, 1100.0]).reading_anchor(),
            Some((2, 60.0))
        );
    }

    #[test]
    fn a_view_at_the_top_is_anchored_on_the_first_card() {
        assert_eq!(place(0.0, &[0.0, 300.0]).reading_anchor(), Some((0, 0.0)));
    }

    #[test]
    fn no_card_on_screen_gives_no_anchor() {
        assert_eq!(place(120.0, &[]).reading_anchor(), None);
    }
}
