use alloc::vec::Vec;

use super::either::Either;

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
    fn add_char(mut self, c: char) -> Option<Either<Self, AnsiSequence>> {
        use Either::*;
        Some(match c {
            'H' => match self.numbers[..] {
                [y, x] => Right(AnsiSequence::CursorPos(x, y)),
                [y] => Right(AnsiSequence::CursorPos(1, y)),
                [] => Right(AnsiSequence::CursorPos(1, 1)),
                _ => return None,
            },

            'A' => Right(AnsiSequence::CursorUp(self.numbers.pop().unwrap_or(1))),
            'B' => Right(AnsiSequence::CursorDown(self.numbers.pop().unwrap_or(1))),
            'C' => Right(AnsiSequence::CursorForward(self.numbers.pop().unwrap_or(1))),
            'D' => Right(AnsiSequence::CursorBackward(
                self.numbers.pop().unwrap_or(1),
            )),

            'J' => Right(AnsiSequence::EraseDisplay),
            'm' => Right(AnsiSequence::SetGraphicsMode(self.numbers)),

            ';' => {
                self.numbers.push(0);
                Left(self)
            }

            '0'..='9' => {
                let digit = c.to_digit(10).unwrap() as u8;

                let Some(number) = self.numbers.last_mut() else {
                    self.numbers.push(digit);
                    return Some(Left(self));
                };

                *number *= 10;
                *number += digit;

                Left(self)
            }
            _ => return None,
        })
    }

    /// Parses the given string into an [`AnsiSequence`]
    /// returns None if the string is not a valid ansi sequence
    /// returns the last index of the sequence and the parsed sequence if successful
    fn parse_seq(chars: &str) -> Option<(usize, AnsiSequence)> {
        let mut chars = chars.chars().enumerate();
        let mut pre_ansi = PreAnsiSequence::new();

        while let Some((i, c)) = chars.next() {
            let parsed = pre_ansi.add_char(c)?;

            if let Either::Right(ansi) = parsed {
                return Some((i, ansi));
            }

            pre_ansi = parsed.unwrap_left();
        }

        None
    }
}

pub struct AnsiiParser<'a> {
    text: &'a str,
}

impl<'a> AnsiiParser<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl<'a> Iterator for AnsiiParser<'a> {
    type Item = Either<AnsiSequence, &'a str>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut chars = self.text.chars().peekable();

        if Some(&'\x1b') == chars.peek() {
            chars.next();
            if Some(&'[') == chars.peek() {
                if let Some((i, seq)) = PreAnsiSequence::parse_seq(&self.text[2..]) {
                    self.text = &self.text[i + 3..];
                    return Some(Either::Left(seq));
                }
            }
        }

        let mut end = 0;
        loop {
            let peek = chars.peek();
            if peek.is_none() && end == 0 {
                break None;
            } else if peek.is_none() || peek.is_some_and(|c| *c == '\x1b') {
                let str = &self.text[..end];
                self.text = &self.text[end..];

                break Some(Either::Right(str));
            }

            chars.next();
            end += 1;
        }
    }
}
