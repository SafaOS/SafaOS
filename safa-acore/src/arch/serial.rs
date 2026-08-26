use crate::arch::{inner, inner::serial::SerialInner};
/// An IO serial that can be used to write kernel logs.
pub struct Serial(SerialInner);

impl Serial {
    /// Initializes the serial IO or returns a dummy debug serial that is later upgraded, note that it currently doesn't support multiple serials only 1 new call should be made...
    ///
    /// This either initializes the serial before MMU and arch is properly initialized or returns a dummy which ignores/handles write_str specially.
    /// A call to [`Self::init_serial`] as an init step.
    pub const fn new() -> Self {
        Self(inner::serial::new_serial())
    }

    /// Initializes the serial.
    ///
    /// the initialization function must not do any serial logging, on error a string is returned and the serial is in proper defined state.
    pub fn init_serial(&mut self) -> Result<(), &'static str> {
        inner::serial::init_serial(&mut self.0)
    }

    /// Write a string to this serial instance
    pub fn write_str(&mut self, s: &str) {
        inner::serial::write_serial_string(&mut self.0, s);
    }
}
