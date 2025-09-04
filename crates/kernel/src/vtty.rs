use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use alloc::sync::Arc;

use crate::{
    drivers::vfs::SeekOffset,
    thread,
    utils::{
        alloc::{PageString, PageVec},
        locks::Mutex,
    },
};

/// A Virtual TTY is similar to PTYs in unix-like systems.
/// It is one of the methods of IPC through a Terminal-like interface,
/// The idea is you got a:
/// - Mother Interface ([`MotherVTTY`]) similar to PTYs Master's interface
/// - Child Interface ([`ChildVTTY`]) similar to PTYs Slave's interface
///
/// A write to a mother interface equals a write to stdin, a read from it equals a read from stdout.
///
/// A write to a child interface equals a write to stdout, a read from it equals a read from stdin.
///
/// The Terminal emulator (or the IPC master) shall use the Mother interface, while the shell for example shall use the child interface as
/// if it is communicating with a TTY.
///
/// Data wrote to this interface must be valid UTF-8 whether it is to stdin or stdout.
#[derive(Debug)]
pub struct VirtualTTY {
    stdout: Mutex<PageString>,
    stdin: Mutex<PageVec<u8>>,
    newlines_count: AtomicUsize,
    should_drop: AtomicBool,
}

impl VirtualTTY {
    const fn new_inner() -> Self {
        Self {
            stdout: Mutex::new(PageString::new()),
            stdin: Mutex::new(PageVec::new()),
            newlines_count: AtomicUsize::new(0),
            should_drop: AtomicBool::new(false),
        }
    }

    /// VirtualTTY is meant to be used in an Arc [`Self::new_inner`] shouldn't be used.
    fn new() -> Arc<Self> {
        Arc::new(Self::new_inner())
    }

    /// Writes a string to stdout
    pub fn write_stdout(&self, s: &str) {
        self.stdout.lock().push_str(s);
    }
    /// Writes a string to stdin
    pub fn write_stdin(&self, s: &str) {
        let bytes = s.as_bytes();
        let mut stdin = self.stdin.lock();

        let newlines = bytes.iter().filter(|c| **c == b'\n').count();
        stdin.reserve(bytes.len());
        stdin.extend_from_slice(bytes);

        self.newlines_count.fetch_add(newlines, Ordering::Release);
    }

    /// Reads `buf`.len() or less bytes from stdout starting from `offset`,
    ///
    /// Returns the amount of data read if successful otherwise Err(()) if the offset is larger or less than the amount of data available.
    pub fn read_stdout(&self, offset: SeekOffset, buf: &mut [u8]) -> Result<usize, ()> {
        let stdout = self.stdout.lock();
        let offset = match offset {
            SeekOffset::End(am) => stdout.len().checked_sub(am).ok_or(())?,
            SeekOffset::Start(s) => s,
        };

        let read_len = buf.len().min(stdout.len().checked_sub(offset).ok_or(())?);
        buf[..read_len].copy_from_slice(&stdout.as_bytes()[..read_len]);
        Ok(read_len)
    }

    /// Reads buf.len() bytes from stdin, may block if there is no data to read, data is defined as a single line, an incomplete line doesn't count as data
    pub fn read_stdin(&self, buf: &mut [u8]) -> usize {
        let mut stdin = self.stdin.lock();

        if self.newlines_count.load(Ordering::Acquire) == 0 {
            drop(stdin);
            thread::current().wait_for_empty_socket(&self.newlines_count, &self.should_drop);
            return self.read_stdin(buf);
        }

        let stdin_bytes = stdin.as_mut_slice();
        let first_newline = stdin_bytes.iter().position(|c| *c == b'\n').unwrap();
        let first_buf_len = first_newline + 1;

        let read_len = buf.len().min(first_buf_len);
        if read_len == first_buf_len {
            self.newlines_count.fetch_sub(1, Ordering::Acquire);
        }

        buf[..read_len].copy_from_slice(&stdin_bytes[..read_len]);
        stdin_bytes.copy_within(read_len.., 0);
        unsafe {
            let stdin_bytes_len = stdin_bytes.len();
            stdin.set_len(stdin_bytes_len - read_len);
        }
        read_len
    }
}

/// A Mother interface over a [`VirtualTTY`].
#[derive(Debug, Clone)]
pub struct MotherVTTY {
    tty: Arc<VirtualTTY>,
}
impl MotherVTTY {
    /// Performs a write operation to stdin
    pub fn write(&self, s: &str) -> usize {
        self.tty.write_stdin(s);
        s.len()
    }
    /// Performs a read operation from stdout
    pub fn read(&self, off: SeekOffset, buf: &mut [u8]) -> Result<usize, ()> {
        self.tty.read_stdout(off, buf)
    }
}

/// A child interface over a [`VirtualTTY`].
#[derive(Debug, Clone)]
pub struct ChildVTTY {
    tty: Arc<VirtualTTY>,
}

impl ChildVTTY {
    /// Performs a write operation to stdout
    pub fn write(&self, s: &str) -> usize {
        self.tty.write_stdout(s);
        s.len()
    }
    /// Performs a read operation from stdin
    pub fn read(&self, buf: &mut [u8]) -> usize {
        self.tty.read_stdin(buf)
    }
}

/// Allocates new VTTY Interfaces, returns a single pair of child and mother interfaces.
pub fn alloc_vtty() -> (MotherVTTY, ChildVTTY) {
    let tty = VirtualTTY::new();
    (MotherVTTY { tty: tty.clone() }, ChildVTTY { tty })
}

#[test_case]
fn vtty_basic_test() {
    const MSG0: &str = "Hello, world!\n";
    const MSG1: &str = ":c\n";

    let mut read_buf = [0u8; 2048];
    let (mother, child) = alloc_vtty();

    child.write(MSG0);

    mother.write(MSG0);
    mother.write(MSG1);

    let len = mother
        .read(SeekOffset::Start(0), &mut read_buf)
        .expect("Failed to read from stdout: InvalidOffset");
    assert_eq!(&read_buf[..len], MSG0.as_bytes());

    let len = child.read(&mut read_buf);
    assert_eq!(&read_buf[..len], MSG0.as_bytes());

    let len = child.read(&mut read_buf);
    assert_eq!(&read_buf[..len], MSG1.as_bytes());
}
