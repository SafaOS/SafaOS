use super::{bstr::BStr, either::Either};

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
    SetGraphicsMode(heapless::Vec<u8, 10>),
}

#[derive(Debug)]
/// A sequence that is parsed into a [`AnsiSequence`]
struct PreAnsiSequence {
    /// the characters that have been parsed so far, separated by ';' and is a u8 number
    numbers: heapless::Vec<u8, 10>,
}

impl PreAnsiSequence {
    fn new() -> Self {
        Self {
            numbers: heapless::Vec::new(),
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
                self.numbers.push(0).unwrap();
                Left(self)
            }

            b'0'..=b'9' => {
                let digit = (c as char).to_digit(10).unwrap() as u8;

                let Some(number) = self.numbers.last_mut() else {
                    self.numbers.push(digit).unwrap();
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
    /// returns the last index of the sequence in `str` and the parsed sequence if successful
    fn parse_seq(str: &BStr) -> Option<(usize, AnsiSequence)> {
        if !matches!(str.get(..2), Some(b"\x1b[")) {
            return None;
        }

        let mut pre_ansi = PreAnsiSequence::new();

        for (i, c) in str[2..].into_iter().enumerate() {
            let parsed = pre_ansi.add_char(*c)?;

            if let Either::Right(ansi) = parsed {
                return Some((i + 2, ansi));
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
        if self.text.is_empty() {
            return None;
        }
        let mut chars = self.text.into_iter().copied().enumerate();

        while let Some((i, c)) = chars.next() {
            if c == b'\x1b' {
                if i == 0 {
                    if let Some((end, seq)) = PreAnsiSequence::parse_seq(&self.text) {
                        self.text = &self.text[end + 1..];
                        return Some(Either::Left(seq));
                    }
                } else {
                    let str = &self.text[..i];
                    self.text = &self.text[i..];
                    return Some(Either::Right(str));
                }
            }
        }

        let str = self.text;
        self.text = BStr::new(b"");
        Some(Either::Right(str))
    }
}
