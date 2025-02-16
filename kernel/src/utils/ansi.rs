use super::{bstr::BStr, either::Either};
use alloc::vec::Vec;

/// A complete ANSII escape sequence
#[derive(Debug)]
pub enum AnsiSequence {
    /// Moves the cursor to the position (x, y)
    CursorPos(u8, u8),

    CursorUp(u8),
    CursorDown(u8),
    CursorForward(u8),
    CursorBackward(u8),

    EraseDisplay,
    SetGraphicsMode(Vec<u8>),
}

#[derive(Debug)]
/// A sequence that is parsed into a [`AnsiSequence`]
struct PreAnsiSequence {
    /// the characters that have been parsed so far, separated by ';' and is a u8 number
    numbers: Vec<u8>,
}

impl PreAnsiSequence {
    fn new() -> Self {
        Self {
            numbers: Vec::with_capacity(5),
        }
    }

    /// Parses the next character into ethier a [`PreAnsiSequence`] or an [`AnsiSequence`] if successful otherwise it returns None if the character is not excepted
    fn add_char(mut self, c: u8) -> Option<Either<Self, AnsiSequence>> {
        use Either::*;
        Some(match c {
            b'H' => match self.numbers[..] {
                [y, x] => Right(AnsiSequence::CursorPos(x, y)),
                [y] => Right(AnsiSequence::CursorPos(1, y)),
                [] => Right(AnsiSequence::CursorPos(1, 1)),
                _ => return None,
            },

            b'A' => Right(AnsiSequence::CursorUp(self.numbers.pop().unwrap_or(1))),
            b'B' => Right(AnsiSequence::CursorDown(self.numbers.pop().unwrap_or(1))),
            b'C' => Right(AnsiSequence::CursorForward(self.numbers.pop().unwrap_or(1))),
            b'D' => Right(AnsiSequence::CursorBackward(
                self.numbers.pop().unwrap_or(1),
            )),

            b'J' => Right(AnsiSequence::EraseDisplay),
            b'm' => Right(AnsiSequence::SetGraphicsMode(self.numbers)),

            b';' => {
                self.numbers.push(0);
                Left(self)
            }

            b'0'..=b'9' => {
                let digit = (c as char).to_digit(10).unwrap() as u8;

                let Some(number) = self.numbers.last_mut() else {
                    self.numbers.push(digit);
                    return Some(Left(self));
                };

                *number = number.saturating_mul(10).saturating_add(digit);
                Left(self)
            }
            _ => return None,
        })
    }

    /// Parses the given string into an [`AnsiSequence`]
    /// returns None if the string is not a valid ansi sequence
    /// returns the last index of the sequence and the parsed sequence if successful
    fn parse_seq(chars: &BStr) -> Option<(usize, AnsiSequence)> {
        let chars = chars.as_bytes().iter().enumerate();
        let mut pre_ansi = PreAnsiSequence::new();

        for (i, c) in chars {
            let parsed = pre_ansi.add_char(*c)?;

            if let Either::Right(ansi) = parsed {
                return Some((i, ansi));
            }

            pre_ansi = parsed.unwrap_left();
        }

        None
    }
}

pub struct AnsiiParser<'a> {
    text: &'a BStr,
}

impl<'a> AnsiiParser<'a> {
    pub fn new(text: &'a BStr) -> Self {
        Self { text }
    }
}

impl<'a> Iterator for AnsiiParser<'a> {
    type Item = Either<AnsiSequence, &'a BStr>;

    // TODO: clean this up
    fn next(&mut self) -> Option<Self::Item> {
        // we are using bytes here because chars are not guaranteed to be valid utf-8 and we need
        // accurate positioning
        let mut chars = self.text;

        if Some(&b'\x1b') == chars.first() {
            if Some(&b'[') == chars.get(1) {
                if let Some((i, seq)) = PreAnsiSequence::parse_seq(&chars[2..]) {
                    self.text = &self.text[i + 3..];
                    return Some(Either::Left(seq));
                }
            }
        }

        let mut end = 0;
        loop {
            if chars.is_empty() && end == 0 {
                break None;
            }

            if chars.is_empty() || chars.first() == Some(&b'\x1b') {
                let str = &self.text[..end];
                self.text = &self.text[end..];
                break Some(Either::Right(str));
            }

            chars = &chars[1..];
            end += 1;
        }
    }
}
