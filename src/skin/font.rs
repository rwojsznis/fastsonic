//! The skin's bitmap font: `text.bmp`, three rows of 5x6 glyphs.
//!
//! The font has capitals only, so lower case maps onto them, and it covers
//! Latin, digits, and a fixed set of punctuation. Accented Latin letters
//! lose their accents rather than turning into question marks; anything the
//! font cannot show at all becomes one.

use super::sprites::{self, Sprite};

/// The blank cell at the end of the first row.
const SPACE: (u32, u32) = (0, 30);

/// The rows of `text.bmp`, in the order the cells appear.
const ROWS: [&str; 3] = [
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ\"@",
    "0123456789\u{2026}.:()-'!_+\\/[]^&%,=$#",
    "\u{c5}\u{d6}\u{c4}?*",
];

/// Where a character sits in the font, as (row, column).
pub fn cell(character: char) -> (u32, u32) {
    let character = fold(character);
    if character == ' ' {
        return SPACE;
    }
    for (row, cells) in ROWS.iter().enumerate() {
        if let Some(column) = cells.chars().position(|cell| cell == character) {
            return (row as u32, column as u32);
        }
    }
    cell('?')
}

/// Whether the font has a real cell for the character, rather than the
/// question mark it falls back to.
pub fn covered(character: char) -> bool {
    character == '?' || cell(character) != cell('?')
}

/// The sprite for a character.
pub fn glyph(character: char) -> Sprite {
    let (row, column) = cell(character);
    sprites::glyph(row, column)
}

/// Reduces a character to one the font has a cell for.
fn fold(character: char) -> char {
    match character {
        'a'..='z' => character.to_ascii_uppercase(),
        ' ' | '\t' | '\u{a0}' => ' ',
        '\u{c5}' | '\u{e5}' => '\u{c5}',
        '\u{d6}' | '\u{f6}' => '\u{d6}',
        '\u{c4}' | '\u{e4}' => '\u{c4}',
        '\u{c0}'..='\u{c3}' | '\u{e0}'..='\u{e3}' => 'A',
        '\u{c6}' | '\u{e6}' => 'A',
        '\u{c7}' | '\u{e7}' => 'C',
        '\u{c8}'..='\u{cb}' | '\u{e8}'..='\u{eb}' => 'E',
        '\u{cc}'..='\u{cf}' | '\u{ec}'..='\u{ef}' => 'I',
        '\u{d0}' | '\u{f0}' => 'D',
        '\u{d1}' | '\u{f1}' => 'N',
        '\u{d2}'..='\u{d5}' | '\u{f2}'..='\u{f5}' | '\u{d8}' | '\u{f8}' => 'O',
        '\u{d9}'..='\u{dc}' | '\u{f9}'..='\u{fc}' => 'U',
        '\u{dd}' | '\u{fd}' | '\u{ff}' => 'Y',
        '\u{df}' => 'S',
        '\u{2018}' | '\u{2019}' | '`' => '\'',
        '\u{201c}' | '\u{201d}' => '"',
        '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
        '<' | '{' => '[',
        '>' | '}' => ']',
        '|' => '/',
        '~' => '-',
        ';' => ',',
        _ => character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::sprites::Sheet;

    #[test]
    fn lower_case_shares_the_capitals() {
        assert_eq!(cell('a'), (0, 0));
        assert_eq!(cell('A'), (0, 0));
        assert_eq!(cell('z'), (0, 25));
        assert_eq!(cell(' '), (0, 30));
    }

    #[test]
    fn digits_and_punctuation_are_on_the_second_row() {
        assert_eq!(cell('0'), (1, 0));
        assert_eq!(cell('9'), (1, 9));
        assert_eq!(cell('.'), (1, 11));
        assert_eq!(cell(':'), (1, 12));
        assert_eq!(cell('('), (1, 13));
        assert_eq!(cell('-'), (1, 15));
        assert_eq!(cell('#'), (1, 30));
    }

    #[test]
    fn accents_fold_and_the_unknown_becomes_a_question_mark() {
        assert_eq!(cell('\u{e9}'), cell('E'));
        assert_eq!(cell('\u{f1}'), cell('N'));
        assert_eq!(cell('\u{e4}'), (2, 2));
        assert_eq!(cell('\u{c5}'), (2, 0));
        assert_eq!(cell('\u{3042}'), (2, 3));
        assert_eq!(cell('?'), (2, 3));
        assert_eq!(cell('\u{2019}'), cell('\''));
    }

    #[test]
    fn a_glyph_is_a_five_by_six_cell_of_the_text_sheet() {
        let sprite = glyph('B');
        assert_eq!(sprite.sheet, Sheet::Text);
        assert_eq!(
            (sprite.x, sprite.y, sprite.width, sprite.height),
            (5, 0, 5, 6)
        );
        assert_eq!(glyph('*').x, 20);
        assert_eq!(glyph('*').y, 12);
    }
}
