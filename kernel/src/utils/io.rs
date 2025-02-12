#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    InvaildOffset,
    InvaildSize,
    Generic,
}

pub trait Readable {
    fn read(&self, offset: isize, buf: &mut [u8]) -> Result<usize, IoError>;
    fn read_exact(&self, offset: isize, buf: &mut [u8]) -> Result<(), IoError> {
        let amount = self.read(offset, buf)?;
        if amount != buf.len() {
            return Err(IoError::InvaildSize);
        }
        Ok(())
    }
}

impl Readable for &[u8] {
    fn read(&self, offset: isize, buf: &mut [u8]) -> Result<usize, IoError> {
        let offset = offset as usize;
        if offset >= self.len() {
            return Err(IoError::InvaildOffset);
        }

        let amount = buf.len().min(self.len() - offset);
        if amount == 0 {
            return Ok(0);
        }

        buf[..amount].copy_from_slice(&self[offset..offset + amount]);
        Ok(amount)
    }
}
