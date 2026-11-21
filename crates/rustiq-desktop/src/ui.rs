use crate::{message::Message, theme};
use iced::{
    Color, Element,
    widget::{row, text},
};

// Font Awesome Free glyphs, bundled so icons render without system fonts.
pub const COPY: char = '\u{f0c5}';
pub const BOX: char = '\u{f466}';
pub const FOLDER: char = '\u{f07b}';
pub const OPEN_FOLDER: char = '\u{f07c}';
pub const REQUEST: char = '\u{f0e7}';
pub const TERMINAL: char = '\u{f120}';
pub const ROCKET: char = '\u{f135}';
pub const EDIT: char = '\u{f303}';
pub const TRASH: char = '\u{f2ed}';
pub const PLUS: char = '\u{f067}';
pub const CHECK: char = '\u{f00c}';
pub const HEART: char = '\u{f004}';
pub const CODE: char = '\u{f121}';
pub const CHEVRON: char = '\u{f054}';
pub const DOWN: char = '\u{f078}';

pub fn icon<'a>(glyph: char, color: Color) -> iced::widget::Text<'a> {
    text(glyph.to_string())
        .font(theme::ICONS)
        .size(14)
        .color(color)
}
pub fn label<'a>(glyph: char, caption: impl Into<String>, color: Color) -> Element<'a, Message> {
    row![icon(glyph, color), text(caption.into()).size(13)]
        .spacing(8)
        .align_y(iced::Center)
        .into()
}
