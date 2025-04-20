use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    InvalidOffset,
    InvalidSize,
    Generic,
}

pub trait Readable {
    fn read(&self, offset: isize, buf: &mut [u8]) -> Result<usize, IoError>;
    fn read_exact(&self, offset: isize, buf: &mut [u8]) -> Result<(), IoError> {
        let amount = self.read(offset, buf)?;
        if amount != buf.len() {
            return Err(IoError::InvalidSize);
        }
        Ok(())
    }
}

impl Readable for &[u8] {
    fn read(&self, offset: isize, buf: &mut [u8]) -> Result<usize, IoError> {
        let offset = offset as usize;
        if offset >= self.len() {
            return Err(IoError::InvalidOffset);
        }

        let amount = buf.len().min(self.len() - offset);
        if amount == 0 {
            return Ok(0);
        }

        buf[..amount].copy_from_slice(&self[offset..offset + amount]);
        Ok(amount)
    }
}

pub struct Cursor<'a> {
    buf: &'a mut [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    pub const fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, offset: 0 }
    }
}

impl<'a> fmt::Write for Cursor<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();

        let remaining = &mut self.buf[self.offset..];
        if remaining.len() < bytes.len() {
            return Err(core::fmt::Error);
        }

        remaining[..bytes.len()].copy_from_slice(bytes);
        self.offset += bytes.len();

        Ok(())
    }
}
