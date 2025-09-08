use core::{
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use alloc::sync::Arc;
use bitflags::bitflags;
use safa_abi::errors::ErrorStatus;

use crate::{
    drivers::vfs::SeekOffset,
    syscalls::ffi::SyscallFFI,
    thread::{self},
    utils::{
        alloc::PageVec,
        locks::{Mutex, RwLock},
    },
};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct TTYFlags: u16 {
        /// Echos stdin to stdout
        const ECHO = 1 << 0;
        /// TODO: write docs
        const CANONICAL = 1 << 1;
        /// Erases the character also from stdout if the erase character was encotured in stdin, (ECHOE), does nothing if not [`TTYFlags::CANONICAL`] and [`TTYFlags::ECHO`].
        const ECHO_ERASE = 1 << 2;
    }
}

#[inline(always)]
const fn ascii_is_ctrl(c: u8) -> bool {
    c <= 0x20 || c == 0x7F
}

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
    stdout: Mutex<PageVec<u8>>,
    stdin: Mutex<PageVec<u8>>,
    flags: RwLock<TTYFlags>,
    newlines_count: AtomicUsize,
    should_drop: AtomicBool,
}

impl VirtualTTY {
    const fn new_inner() -> Self {
        Self {
            stdout: Mutex::new(PageVec::new()),
            stdin: Mutex::new(PageVec::new()),
            flags: RwLock::new(TTYFlags::CANONICAL.union(TTYFlags::ECHO)),
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
        let bytes = s.as_bytes();
        let mut stdout = self.stdout.lock();
        stdout.reserve(bytes.len());
        stdout.extend_from_slice(bytes);
    }
    /// Writes a string to stdin
    pub fn write_stdin(&self, s: &str) {
        const ERASE: u8 = 0x7f;

        let bytes = s.as_bytes();
        let mut stdin = self.stdin.lock();
        let stdin = &mut *stdin;

        let flags = self.flags.read();
        let mut stdout = flags.contains(TTYFlags::ECHO).then(|| self.stdout.lock());

        let mut echo = move |c: u8| {
            if let Some(ref mut s) = stdout {
                s.push(c)
            }
        };

        macro_rules! echo_ctrl {
            ($c: expr) => {{
                echo(b'^');
                echo($c);
            }};
        }

        macro_rules! echo_erase {
            () => {{
                echo(b'\x08');
                echo(b' ');
                echo(b'\x08');
            }};
        }

        let mut newlines_count = self.newlines_count.load(Ordering::Acquire);

        for b in bytes {
            match *b {
                ERASE if flags.contains(TTYFlags::CANONICAL) => {
                    if stdin.last().is_none_or(|c| c == &b'\n') {
                        continue;
                    }

                    let c = stdin.pop().unwrap();
                    let c_was_ctrl = ascii_is_ctrl(c);

                    if flags.contains(TTYFlags::ECHO) {
                        if flags.contains(TTYFlags::ECHO_ERASE) {
                            // echo '\b'
                            echo_erase!();
                            if c_was_ctrl {
                                echo_erase!()
                            }
                        } else {
                            echo_ctrl!(b'H');
                        }
                    }
                }
                o => {
                    stdin.push(o);
                    echo(o);

                    if o == b'\n' {
                        newlines_count += 1;
                    }
                }
            }
        }
        self.newlines_count.store(newlines_count, Ordering::Release);
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
        buf[..read_len].copy_from_slice(&stdout[offset..offset + read_len]);
        Ok(read_len)
    }

    /// Reads buf.len() bytes from stdin, may block if there is no data to read, data is defined as a single line, an incomplete line doesn't count as data
    pub fn read_stdin(&self, buf: &mut [u8]) -> usize {
        let mut stdin = self.stdin.lock();
        let canonical = self.flags.read().contains(TTYFlags::CANONICAL);

        if canonical && self.newlines_count.load(Ordering::Acquire) == 0 {
            drop(stdin);
            thread::current().wait_for_empty_socket(&self.newlines_count, &self.should_drop);
            return self.read_stdin(buf);
        }

        let stdin_bytes = stdin.as_mut_slice();
        let max_read = if canonical {
            let first_newline = stdin_bytes.iter().position(|c| *c == b'\n').unwrap();
            first_newline + 1
        } else {
            stdin_bytes.len()
        };

        let read_len = buf.len().min(max_read);
        if canonical && read_len == max_read {
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

    pub fn set_flags(&self, tty_flags: TTYFlags) {
        *self.flags.write() = tty_flags;
    }

    pub fn read_flags(&self) -> TTYFlags {
        *self.flags.read()
    }

    pub fn process_command(&self, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
        const GET_FLAGS: u16 = 0;
        const SET_FLAGS: u16 = 1;

        match cmd {
            GET_FLAGS => {
                let arg: &mut TTYFlags = SyscallFFI::make(arg as *mut TTYFlags)?;
                *arg = self.read_flags();
                Ok(())
            }
            SET_FLAGS => {
                let arg = arg as u16;
                let flags = TTYFlags::from_bits_retain(arg);
                self.set_flags(flags);
                Ok(())
            }
            _ => Err(ErrorStatus::InvalidCommand),
        }
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

impl Deref for MotherVTTY {
    type Target = VirtualTTY;
    fn deref(&self) -> &Self::Target {
        &*self.tty
    }
}

impl Deref for ChildVTTY {
    type Target = VirtualTTY;
    fn deref(&self) -> &Self::Target {
        &*self.tty
    }
}

/// Allocates new VTTY Interfaces, returns a single pair of child and mother interfaces.
pub fn alloc_vtty() -> (MotherVTTY, ChildVTTY) {
    let tty = VirtualTTY::new();
    (MotherVTTY { tty: tty.clone() }, ChildVTTY { tty })
}

#[allow(unused_assignments)]
#[test_case]
fn vtty_canonical_mode_test() {
    const MSG0: &str = "Hello, world!\n";
    const SPEC_MSG: &str = "hi\x7flol\n";
    const SPEC_MSG_REPLY: &str = "hlol\n";
    const SPEC_MSG_REPLY_STDOUT0: &str = "hi^Hlol\n";
    const SPEC_MSG_REPLY_STDOUT1: &str = "hi\x08 \x08lol\n";
    const MSG1: &str = ":c\n";

    let mut read_buf = [0u8; 2048];
    let (mother, child) = alloc_vtty();

    mother.set_flags(TTYFlags::CANONICAL | TTYFlags::ECHO);
    child.write(MSG0);
    mother.write(MSG0);
    mother.write(SPEC_MSG);
    mother.set_flags(TTYFlags::CANONICAL | TTYFlags::ECHO | TTYFlags::ECHO_ERASE);
    mother.write(SPEC_MSG);
    mother.write(MSG1);
    // No echo
    mother.set_flags(TTYFlags::CANONICAL);
    mother.write(MSG1);

    let mut mo_off = 0;
    macro_rules! assert_child {
        ($buf: ident, $c: ident, $o: expr) => {{
            let read_buf = &mut $buf;
            let child = &$c;
            let len = child.read(read_buf);
            let read = &read_buf[..len];
            unsafe {
                assert_eq!(
                    read,
                    $o as &[u8],
                    "read: '{}', expected: '{}'; from child",
                    str::from_utf8_unchecked(read),
                    str::from_utf8_unchecked($o as &[u8])
                );
            }
        }};
        ($o: expr) => {
            assert_child!(read_buf, child, $o)
        };
    }

    macro_rules! assert_mother {
        ($o: expr) => {{
            let o = $o as &[u8];
            let len = mother
                .read(SeekOffset::Start(mo_off), &mut read_buf[..o.len()])
                .expect("Failed to read from stdout: InvalidOffset");
            let read = &read_buf[..len];
            unsafe {
                assert_eq!(
                    read,
                    o,
                    "read: '{}', expected: '{}'; from mother",
                    str::from_utf8_unchecked(read),
                    str::from_utf8_unchecked(o)
                );
            }
            mo_off += len;
        }};
    }

    assert_mother!(MSG0.as_bytes());
    // Echoed
    assert_mother!(MSG0.as_bytes());
    assert_mother!(SPEC_MSG_REPLY_STDOUT0.as_bytes());
    assert_mother!(SPEC_MSG_REPLY_STDOUT1.as_bytes());
    assert_mother!(MSG1.as_bytes());
    // we wrote MSG1 but without echoing
    assert_mother!(&[]);

    assert_child!(MSG0.as_bytes());
    assert_child!(SPEC_MSG_REPLY.as_bytes());
    assert_child!(SPEC_MSG_REPLY.as_bytes());
    assert_child!(MSG1.as_bytes());
    assert_child!(MSG1.as_bytes());

    // Testing blocking
    use crate::arch::with_interrupts;
    use crate::process::current::kernel_thread_spawn;
    use crate::thread::Tid;
    use core::cell::SyncUnsafeCell;
    use core::mem::MaybeUninit;

    static CHILD: SyncUnsafeCell<MaybeUninit<ChildVTTY>> =
        SyncUnsafeCell::new(MaybeUninit::uninit());
    static WROTE: AtomicBool = AtomicBool::new(false);

    unsafe { *CHILD.get() = MaybeUninit::new(child.clone()) };

    fn test_thread(_: Tid, _: &'static ()) -> ! {
        let mut read_buf = [0u8; 2048];
        let child = unsafe { (*CHILD.get()).assume_init_read() };

        assert_child!(read_buf, child, MSG0.as_bytes());
        WROTE.store(true, Ordering::Release);
        thread::current::exit(0);
    }
    with_interrupts(|| {
        kernel_thread_spawn(test_thread, &(), None, None).expect("Failed to spawn test thread");
        mother.write(MSG0);
        while !WROTE.load(Ordering::Acquire) {}
    });
}
