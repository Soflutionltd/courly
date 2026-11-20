//! The box of a table cell, read from the HTML attributes a table keeps:
//! paddings, borders, background, sizes, alignment. And the widths the
//! columns of a table take.
//!
//! Lengths are CSS pixels. What is not understood is left at its default:
//! the attributes themselves are written back unchanged, so the reader of
//! the mail still gets them.

use crate::document::{HtmlAttrs, Rgb};

/// Padding of a cell that asks for none, as HTML has it (px).
const DEFAULT_CELL_PADDING: f32 = 1.0;
/// Space between cells of a table that asks for none, as HTML has it (px).
const DEFAULT_CELL_SPACING: f32 = 2.0;
/// Width of a border whose width is `medium` or not given (px).
const MEDIUM_BORDER: f32 = 3.0;
/// Narrowest a column is squeezed to (px).
pub const MIN_COLUMN_WIDTH: f32 = 16.0;

/// One value per side of a box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    /// The same value on every side.
    pub const fn all(value: T) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// The sides a CSS shorthand of one to four values stands for.
    pub fn shorthand(values: &[T]) -> Option<Self> {
        match *values {
            [all] => Some(Self::all(all)),
            [vertical, horizontal] => Some(Self {
                top: vertical,
                right: horizontal,
                bottom: vertical,
                left: horizontal,
            }),
            [top, horizontal, bottom] => Some(Self {
                top,
                right: horizontal,
                bottom,
                left: horizontal,
            }),
            [top, right, bottom, left] => Some(Self {
                top,
                right,
                bottom,
                left,
            }),
            _ => None,
        }
    }
}

/// A visible border line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderLine {
    pub width: f32,
    /// `None` for the color of the text.
    pub color: Option<Rgb>,
}

/// A width asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoxWidth {
    Pixels(f32),
    /// Percent of the width of what holds the box.
    Percent(f32),
}

impl BoxWidth {
    /// The width in pixels, inside something `container` wide.
    pub fn pixels(self, container: f32) -> f32 {
        match self {
            Self::Pixels(pixels) => pixels,
            Self::Percent(percent) => container * percent / 100.0,
        }
    }
}

/// Where the content of a cell sits when the row is taller than it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// The box of a cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellBox {
    pub padding: Sides<f32>,
    pub border: Sides<Option<BorderLine>>,
    pub background: Option<Rgb>,
    pub width: Option<BoxWidth>,
    /// Least height of the cell.
    pub height: Option<f32>,
    pub vertical: VerticalAlign,
    /// Number of columns the cell takes.
    pub span: usize,
}

impl CellBox {
    /// Width taken by the left and right paddings and borders.
    pub fn horizontal_extra(&self) -> f32 {
        self.padding.left
            + self.padding.right
            + border_width(self.border.left)
            + border_width(self.border.right)
    }

    /// Height taken by the top and bottom paddings and borders.
    pub fn vertical_extra(&self) -> f32 {
        self.padding.top
            + self.padding.bottom
            + border_width(self.border.top)
            + border_width(self.border.bottom)
    }

    /// How far the content starts from the left edge of the cell.
    pub fn content_left(&self) -> f32 {
        self.padding.left + border_width(self.border.left)
    }

    /// How far the content starts from the top edge of the cell.
    pub fn content_top(&self) -> f32 {
        self.padding.top + border_width(self.border.top)
    }
}

fn border_width(line: Option<BorderLine>) -> f32 {
    line.map_or(0.0, |line| line.width)
}

/// A CSS length in pixels: `px`, `pt`, `cm`, `mm`, `in`, or a bare number
/// as HTML attributes write them. Relative units are not read.
pub fn css_length(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let (number, factor) = [
        ("px", 1.0),
        ("pt", 96.0 / 72.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("in", 96.0),
    ]
    .iter()
    .find_map(|(unit, factor)| Some((value.strip_suffix(*unit)?, *factor)))
    .unwrap_or((value.as_str(), 1.0));
    let number = number.trim().parse::<f32>().ok()?;
    (number.is_finite() && number >= 0.0).then_some(number * factor)
}

/// A width: a length, or a percentage.
pub fn css_width(value: &str) -> Option<BoxWidth> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<f32>().ok()?;
        return (percent.is_finite() && percent >= 0.0).then_some(BoxWidth::Percent(percent));
    }
    css_length(value).map(BoxWidth::Pixels)
}

/// The values of a CSS property, split at the white space outside
/// parentheses, so that `rgb(1, 2, 3)` stays one value.
pub fn css_values(value: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut depth = 0_usize;
    let mut start = None;
    for (index, c) in value.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c.is_whitespace() && depth == 0 {
            if let Some(from) = start.take() {
                values.push(&value[from..index]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(from) = start {
        values.push(&value[from..]);
    }
    values
}

/// What the declarations seen so far say about one border side.
#[derive(Debug, Clone, Copy, Default)]
struct BorderSide {
    width: Option<f32>,
    visible: Option<bool>,
    color: Option<Rgb>,
}

impl BorderSide {
    fn line(self) -> Option<BorderLine> {
        self.visible.unwrap_or(false).then(|| BorderLine {
            width: self.width.unwrap_or(MEDIUM_BORDER),
            color: self.color,
        })
    }

    /// Take a `border` or `border-left` shorthand: a width, a style and a
    /// color in any order.
    fn take_shorthand(&mut self, value: &str) {
        for part in css_values(value) {
            if let Some(width) = border_width_value(part) {
                self.width = Some(width);
            } else if let Some(visible) = border_style_value(part) {
                self.visible = Some(visible);
            } else if let Some(color) = Rgb::parse_css(part) {
                self.color = Some(color);
            }
        }
    }
}

fn border_width_value(value: &str) -> Option<f32> {
    match value.to_ascii_lowercase().as_str() {
        "thin" => Some(1.0),
        "medium" => Some(MEDIUM_BORDER),
        "thick" => Some(5.0),
        _ => css_length(value),
    }
}

/// Whether a `border-style` keyword draws a line. `None` for a value that
/// is not a border style.
fn border_style_value(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "none" | "hidden" => Some(false),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(true)
        }
        _ => None,
    }
}

/// Apply a one-to-four value property to the four sides, in the order top,
/// right, bottom, left. A value that is not read leaves its side alone.
fn each_side<T: Copy>(
    sides: &mut [BorderSide; 4],
    value: &str,
    read: impl Fn(&str) -> Option<T>,
    set: impl Fn(&mut BorderSide, T),
) {
    let values: Vec<Option<T>> = css_values(value).into_iter().map(read).collect();
    let Some(Sides {
        top,
        right,
        bottom,
        left,
    }) = Sides::shorthand(&values)
    else {
        return;
    };
    for (side, value) in sides.iter_mut().zip([top, right, bottom, left]) {
        if let Some(value) = value {
            set(side, value);
        }
    }
}

/// The `(property, value)` declarations of a `style` attribute, in order.
fn declarations(attrs: &HtmlAttrs) -> impl Iterator<Item = (String, &str)> {
    attrs
        .get("style")
        .unwrap_or_default()
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(property, value)| {
            (
                property.trim().to_ascii_lowercase(),
                value.trim().trim_end_matches("!important").trim(),
            )
        })
}

/// The box of a cell, from its own attributes, those of its row and those
/// of its table.
pub fn cell_box(table: &HtmlAttrs, row: &HtmlAttrs, cell: &HtmlAttrs) -> CellBox {
    let table_padding = table
        .get("cellpadding")
        .and_then(css_length)
        .unwrap_or(DEFAULT_CELL_PADDING);
    let mut padding = Sides::all(table_padding);

    // Sides in the order top, right, bottom, left. A table with a `border`
    // attribute gives each of its cells a thin line.
    let framed = table
        .get("border")
        .and_then(css_length)
        .is_some_and(|width| width > 0.0);
    let mut border = [BorderSide {
        width: framed.then_some(1.0),
        visible: framed.then_some(true),
        color: None,
    }; 4];

    let mut background = None;
    let mut width = cell.get("width").and_then(css_width);
    let mut height = cell
        .get("height")
        .and_then(css_length)
        .or_else(|| row.get("height").and_then(css_length))
        .or_else(|| row.style("height").and_then(css_length));
    let mut vertical = cell
        .get("valign")
        .or_else(|| row.get("valign"))
        .and_then(vertical_align);

    for (property, value) in declarations(cell) {
        match property.as_str() {
            "padding" => {
                let values: Option<Vec<f32>> =
                    css_values(value).into_iter().map(css_length).collect();
                if let Some(sides) = values.and_then(|values| Sides::shorthand(&values)) {
                    padding = sides;
                }
            }
            "padding-top" => padding.top = css_length(value).unwrap_or(padding.top),
            "padding-right" => padding.right = css_length(value).unwrap_or(padding.right),
            "padding-bottom" => padding.bottom = css_length(value).unwrap_or(padding.bottom),
            "padding-left" => padding.left = css_length(value).unwrap_or(padding.left),
            "border" => border
                .iter_mut()
                .for_each(|side| side.take_shorthand(value)),
            "border-top" => border[0].take_shorthand(value),
            "border-right" => border[1].take_shorthand(value),
            "border-bottom" => border[2].take_shorthand(value),
            "border-left" => border[3].take_shorthand(value),
            "border-width" => each_side(&mut border, value, border_width_value, |side, width| {
                side.width = Some(width);
            }),
            "border-style" => each_side(&mut border, value, border_style_value, |side, visible| {
                side.visible = Some(visible);
            }),
            "border-color" => each_side(&mut border, value, Rgb::parse_css, |side, color| {
                side.color = Some(color);
            }),
            "background-color" | "background" => {
                background = Rgb::parse_css(value).or(background);
            }
            "width" => width = css_width(value).or(width),
            "height" => height = css_length(value).or(height),
            "vertical-align" => vertical = vertical_align(value).or(vertical),
            _ => {}
        }
    }

    let background = background
        .or_else(|| cell.get("bgcolor").and_then(Rgb::parse_css))
        .or_else(|| row.style("background-color").and_then(Rgb::parse_css))
        .or_else(|| row.get("bgcolor").and_then(Rgb::parse_css));

    CellBox {
        padding,
        border: Sides {
            top: border[0].line(),
            right: border[1].line(),
            bottom: border[2].line(),
            left: border[3].line(),
        },
        background,
        width,
        height,
        vertical: vertical.unwrap_or_default(),
        span: cell
            .get("colspan")
            .and_then(|span| span.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .max(1),
    }
}

fn vertical_align(value: &str) -> Option<VerticalAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
        "bottom" => Some(VerticalAlign::Bottom),
        _ => None,
    }
}

/// The space between the cells of a table (px).
pub fn table_spacing(table: &HtmlAttrs) -> f32 {
    if let Some(spacing) = table.get("cellspacing").and_then(css_length) {
        return spacing;
    }
    if table
        .style("border-collapse")
        .is_some_and(|value| value.eq_ignore_ascii_case("collapse"))
    {
        return 0.0;
    }
    table
        .style("border-spacing")
        .and_then(|value| css_values(value).first().copied().and_then(css_length))
        .unwrap_or(DEFAULT_CELL_SPACING)
}

/// The width a table asks for.
pub fn table_width(table: &HtmlAttrs) -> Option<BoxWidth> {
    table
        .style("width")
        .and_then(css_width)
        .or_else(|| table.get("width").and_then(css_width))
}

/// The background a table asks for.
pub fn table_background(table: &HtmlAttrs) -> Option<Rgb> {
    table
        .style("background-color")
        .and_then(Rgb::parse_css)
        .or_else(|| table.get("bgcolor").and_then(Rgb::parse_css))
}

/// The widths of the columns of a table.
///
/// `fixed` holds the width a column asks for, `natural` the width its
/// content takes on one line. A column without a fixed width starts at its
/// natural one. The table is then brought to `table_width` when it asks for
/// one, and in every case inside `available`: space to give or to take
/// goes to the columns without a fixed width first, in proportion to their
/// widths.
pub fn column_widths(
    fixed: &[Option<f32>],
    natural: &[f32],
    table_width: Option<f32>,
    available: f32,
) -> Vec<f32> {
    let mut widths: Vec<f32> = fixed
        .iter()
        .zip(natural)
        .map(|(fixed, natural)| fixed.unwrap_or(*natural).max(0.0))
        .collect();
    let total: f32 = widths.iter().sum();
    let target = match table_width {
        Some(asked) if available >= 1.0 => asked.min(available),
        Some(asked) => asked,
        None if available >= 1.0 && total > available => available,
        None => return widths,
    };
    if (total - target).abs() < 0.01 {
        return widths;
    }

    let free: Vec<usize> = (0..widths.len())
        .filter(|index| fixed[*index].is_none())
        .collect();
    let every: Vec<usize> = (0..widths.len()).collect();
    if total < target {
        let group = if free.is_empty() { &every } else { &free };
        grow(&mut widths, group, target - total);
        return widths;
    }
    let over = shrink(&mut widths, &free, total - target);
    if over > 0.01 {
        shrink(&mut widths, &every, over);
    }
    widths
}

/// Share `extra` among the columns of `group`, in proportion to their
/// widths, or evenly when they have none.
fn grow(widths: &mut [f32], group: &[usize], extra: f32) {
    let total: f32 = group.iter().map(|index| widths[*index]).sum();
    for index in group {
        let share = if total > 0.0 {
            widths[*index] / total
        } else {
            1.0 / group.len() as f32
        };
        widths[*index] += extra * share;
    }
}

/// Take `excess` from the columns of `group`, in proportion to what each
/// has above [`MIN_COLUMN_WIDTH`]. Returns what could not be taken.
fn shrink(widths: &mut [f32], group: &[usize], excess: f32) -> f32 {
    let room: f32 = group
        .iter()
        .map(|index| (widths[*index] - MIN_COLUMN_WIDTH).max(0.0))
        .sum();
    if room <= 0.0 {
        return excess;
    }
    let taken = excess.min(room);
    for index in group {
        let spare = (widths[*index] - MIN_COLUMN_WIDTH).max(0.0);
        widths[*index] -= taken * spare / room;
    }
    excess - taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> HtmlAttrs {
        HtmlAttrs::new(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    fn rounded(values: &[f32]) -> Vec<i64> {
        values
            .iter()
            .map(|value| format!("{value:.0}").parse::<i64>().expect("whole number"))
            .collect()
    }

    #[test]
    fn lengths_are_read_in_pixels() {
        assert_eq!(css_length("12px"), Some(12.0));
        assert_eq!(css_length(" 9PT "), Some(12.0));
        assert_eq!(css_length("1in"), Some(96.0));
        assert_eq!(css_length("0cm"), Some(0.0));
        assert_eq!(css_length("40"), Some(40.0));
        assert_eq!(css_length("2em"), None);
        assert_eq!(css_length("-4px"), None);
        assert_eq!(css_length("medium"), None);
        assert_eq!(css_width("50%"), Some(BoxWidth::Percent(50.0)));
        assert_eq!(css_width("150pt"), Some(BoxWidth::Pixels(200.0)));
    }

    #[test]
    fn values_split_outside_parentheses() {
        assert_eq!(
            css_values("1pt solid rgb(100, 100, 100)"),
            ["1pt", "solid", "rgb(100, 100, 100)"]
        );
        assert_eq!(css_values("  0cm   13.5pt "), ["0cm", "13.5pt"]);
        assert!(css_values("").is_empty());
    }

    #[test]
    fn a_plain_cell_takes_the_defaults_of_html() {
        let plain = cell_box(&attrs(&[]), &attrs(&[]), &attrs(&[]));
        assert_eq!(plain.padding, Sides::all(1.0));
        assert_eq!(plain.border, Sides::all(None));
        assert_eq!(plain.vertical, VerticalAlign::Middle);
        assert_eq!(plain.span, 1);
        assert_eq!(plain.width, None);
        assert_eq!(Some(table_spacing(&attrs(&[]))), Some(2.0));
    }

    #[test]
    fn padding_shorthand_and_table_attributes_are_read() {
        let table = attrs(&[("cellpadding", "0"), ("cellspacing", "0"), ("border", "0")]);
        let cell = attrs(&[("style", "padding:0cm 13.5pt 0cm 0.75pt; width:150pt")]);
        let read = cell_box(&table, &attrs(&[]), &cell);
        assert_eq!(
            read.padding,
            Sides {
                top: 0.0,
                right: 18.0,
                bottom: 0.0,
                left: 1.0
            }
        );
        assert_eq!(read.width, Some(BoxWidth::Pixels(200.0)));
        assert_eq!(read.border, Sides::all(None));

        // Without a style, the cell takes the padding of its table.
        let bare = cell_box(&attrs(&[("cellpadding", "6")]), &attrs(&[]), &attrs(&[]));
        assert_eq!(bare.padding, Sides::all(6.0));
    }

    #[test]
    fn one_sided_border_written_as_three_properties_is_read() {
        let cell = attrs(&[(
            "style",
            "border-width:medium medium medium 1pt; border-style:none none none solid; \
             border-color:currentcolor currentcolor currentcolor rgb(200, 200, 200); \
             padding:0cm 0cm 0cm 13.5pt; vertical-align:top",
        )]);
        let read = cell_box(&attrs(&[]), &attrs(&[]), &cell);
        assert_eq!(read.border.top, None);
        assert_eq!(read.border.right, None);
        assert_eq!(read.border.bottom, None);
        // A keyword that is not read leaves its side alone; the left side
        // still takes its color.
        let left = read.border.left.expect("left border");
        assert_eq!(rounded(&[left.width * 3.0]), [4]);
        assert_eq!(left.color, Some(Rgb::new(200, 200, 200)));
        assert_eq!(read.vertical, VerticalAlign::Top);
    }

    #[test]
    fn border_shorthand_and_table_frame_are_read() {
        let cell = attrs(&[("style", "border: 2px solid #336699; border-top: none")]);
        let read = cell_box(&attrs(&[]), &attrs(&[]), &cell);
        assert_eq!(read.border.top, None);
        assert_eq!(
            read.border.left,
            Some(BorderLine {
                width: 2.0,
                color: Some(Rgb::new(0x33, 0x66, 0x99))
            })
        );

        let framed = cell_box(&attrs(&[("border", "1")]), &attrs(&[]), &attrs(&[]));
        assert_eq!(
            framed.border,
            Sides::all(Some(BorderLine {
                width: 1.0,
                color: None
            }))
        );
    }

    #[test]
    fn height_background_and_span_fall_back_on_the_row() {
        let row = attrs(&[
            ("height", "30"),
            ("bgcolor", "#ff0000"),
            ("valign", "bottom"),
        ]);
        let cell = attrs(&[("colspan", "2")]);
        let read = cell_box(&attrs(&[]), &row, &cell);
        assert_eq!(read.height, Some(30.0));
        assert_eq!(read.background, Some(Rgb::new(0xff, 0, 0)));
        assert_eq!(read.vertical, VerticalAlign::Bottom);
        assert_eq!(read.span, 2);

        let own = attrs(&[("style", "height:36pt; background-color:#00ff00")]);
        let read = cell_box(&attrs(&[]), &row, &own);
        assert_eq!(read.height, Some(48.0));
        assert_eq!(read.background, Some(Rgb::new(0, 0xff, 0)));
    }

    #[test]
    fn collapsed_table_has_no_spacing_and_its_width_is_read() {
        let table = attrs(&[("style", "width:520.5px; border-collapse:collapse")]);
        assert_eq!(Some(table_spacing(&table)), Some(0.0));
        assert_eq!(table_width(&table), Some(BoxWidth::Pixels(520.5)));
        assert_eq!(
            table_width(&attrs(&[("width", "100%")])),
            Some(BoxWidth::Percent(100.0))
        );
    }

    #[test]
    fn columns_keep_their_natural_width_when_nothing_asks_otherwise() {
        let widths = column_widths(&[None, None], &[120.0, 80.0], None, 600.0);
        assert_eq!(rounded(&widths), [120, 80]);
    }

    #[test]
    fn extra_width_goes_to_the_columns_without_a_fixed_width() {
        let widths = column_widths(
            &[Some(200.0), None, None],
            &[0.0, 100.0, 50.0],
            Some(500.0),
            600.0,
        );
        assert_eq!(rounded(&widths), [200, 200, 100]);

        // Every column fixed: all of them stretch.
        let widths = column_widths(&[Some(100.0), Some(300.0)], &[0.0, 0.0], Some(600.0), 600.0);
        assert_eq!(rounded(&widths), [150, 450]);
    }

    #[test]
    fn a_table_too_wide_is_squeezed_from_its_free_columns_first() {
        let widths = column_widths(&[Some(200.0), None], &[0.0, 500.0], None, 400.0);
        assert_eq!(rounded(&widths), [200, 200]);

        // The free column cannot go under the narrowest width: the fixed
        // one gives the rest.
        let widths = column_widths(&[Some(400.0), None], &[0.0, 100.0], None, 216.0);
        assert_eq!(rounded(&widths), [200, 16]);
    }
}
