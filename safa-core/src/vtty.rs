use core::ops::Deref;

use alloc::sync::Arc;
use bitflags::bitflags;
use safa_abi::{errors::ErrorStatus, poll::PollEvents};

use crate::{
    drivers::vfs::SeekOffset,
    process::{
        poll::{self, PollID},
        resources::{self, Resource},
    },
    scheduler::wait_queue::{WaitError, WaitQueue},
    syscalls::ffi::SyscallFFI,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitReason {
    CanonicalRead,
    RawRead,
}

#[derive(Debug)]
struct Stdin {
    inner: PageVec<u8>,
    newlines_count: usize,
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
    stdin: Mutex<Stdin>,
    flags: RwLock<TTYFlags>,
    wait_queue: Mutex<WaitQueue<2, WaitReason>>,
}

impl VirtualTTY {
    const fn new_inner() -> Self {
        Self {
            stdout: Mutex::new(PageVec::new(&"VirtualTTY::Stdout")),
            stdin: Mutex::new(Stdin {
                inner: PageVec::new(&"VirtualTTY::Stdin"),
                newlines_count: 0,
            }),
            flags: RwLock::new(TTYFlags::CANONICAL.union(TTYFlags::ECHO)),
            wait_queue: Mutex::new(WaitQueue::new()),
        }
    }

    /// VirtualTTY is meant to be used in an Arc [`Self::new_inner`] shouldn't be used.
    fn new() -> Arc<Self> {
        Arc::new(Self::new_inner())
    }

    /// Writes a string to stdout
    pub fn write_stdout(&self, s: &str) {
        if s.len() == 0 {
            return;
        }
        let bytes = s.as_bytes();
        self.write_stdout_inner(&mut self.stdout.lock(), bytes)
    }

    fn write_stdout_inner(&self, stdout: &mut PageVec<u8>, bytes: &[u8]) {
        if bytes.len() == 0 {
            return;
        }
        let data_was_ava = !stdout.is_empty();

        stdout.reserve(bytes.len());
        stdout.extend_from_slice(bytes);
        if !data_was_ava {
            /* targets mother poll */
            poll::broadcast_events(
                self.mother_poll_id(),
                PollEvents::DATA_AVAILABLE,
                PollEvents::NONE,
            );
        }
    }
    /// Writes a string to stdin
    pub fn write_stdin(&self, s: &str) {
        if s.len() == 0 {
            return;
        }

        const ERASE: u8 = 0x7f;

        let bytes = s.as_bytes();
        let mut stdin = self.stdin.lock();
        let stdin = &mut *stdin;

        let flags = self.flags.read();
        let mut stdout = flags.contains(TTYFlags::ECHO).then(|| self.stdout.lock());

        let mut echo = move |s: &[u8]| {
            if let Some(ref mut stdout) = stdout {
                self.write_stdout_inner(stdout, s);
            }
        };

        macro_rules! echo_ctrl {
            ($c: expr) => {{
                echo(b"^");
                echo($c.as_bytes());
            }};
        }

        macro_rules! echo_erase {
            () => {{
                echo(b"\x08 \x08");
            }};
        }

        let newlines_count_was = stdin.newlines_count;
        let stdin_had = stdin.inner.len();

        for b in bytes {
            match *b {
                ERASE if flags.contains(TTYFlags::CANONICAL) => {
                    if stdin.inner.last().is_none_or(|c| c == &b'\n') {
                        continue;
                    }

                    let c = stdin.inner.pop().unwrap();
                    let c_was_ctrl = ascii_is_ctrl(c);

                    if flags.contains(TTYFlags::ECHO) {
                        if flags.contains(TTYFlags::ECHO_ERASE) {
                            // echo '\b'
                            echo_erase!();
                            if c_was_ctrl {
                                echo_erase!()
                            }
                        } else {
                            echo_ctrl!("H");
                        }
                    }
                }
                o => {
                    stdin.inner.push(o);
                    echo(&[o]);

                    if o == b'\n' {
                        stdin.newlines_count += 1;
                    }
                }
            }
        }

        if flags.contains(TTYFlags::CANONICAL) && stdin.newlines_count > 0 {
            self.wait_queue.lock().wake_on_condition(|r| {
                *r == WaitReason::CanonicalRead || *r == WaitReason::RawRead
            });
        } else if !stdin.inner.is_empty() && !flags.contains(TTYFlags::CANONICAL) {
            self.wait_queue.lock().wake_on_condition(|r| {
                *r == WaitReason::CanonicalRead || *r == WaitReason::RawRead
            });
        }

        if (flags.contains(TTYFlags::CANONICAL)
            && newlines_count_was == 0
            && stdin.newlines_count > 0)
            || (!flags.contains(TTYFlags::CANONICAL) && stdin_had == 0)
        {
            poll::broadcast_events(
                self.child_poll_id(),
                PollEvents::DATA_AVAILABLE,
                PollEvents::NONE,
            );
        }
    }

    /// Reads `buf`.len() or less bytes from stdout,
    ///
    /// Returns the amount of data read if successful otherwise Err(()) if the offset is larger or less than the amount of data available.
    pub fn read_stdout(&self, buf: &mut [u8]) -> Result<usize, ()> {
        let mut stdout = self.stdout.lock();

        let stdout_len = stdout.len();
        let read_len = buf.len().min(stdout_len);

        buf[..read_len].copy_from_slice(&stdout[..read_len]);

        stdout.copy_within(read_len.., 0);
        stdout.truncate(stdout_len - read_len);

        if stdout.is_empty() {
            poll::broadcast_events(
                self.mother_poll_id(),
                PollEvents::NONE,
                PollEvents::DATA_AVAILABLE,
            );
        }
        Ok(read_len)
    }

    /// Reads buf.len() bytes from stdin, may block if there is no data to read, data is defined as a single line, an incomplete line doesn't count as data
    pub fn read_stdin(&self, buf: &mut [u8]) -> Result<usize, WaitError> {
        let mut stdin = self.stdin.lock();
        let is_canonical = self.flags.read().contains(TTYFlags::CANONICAL);

        if is_canonical {
            if stdin.newlines_count == 0 {
                let pending = self.wait_queue.prepare_wait();
                drop(stdin);
                pending.enter_wait(WaitReason::CanonicalRead, None)?;

                return self.read_stdin(buf);
            }
        } else if stdin.inner.is_empty() && !buf.is_empty() {
            let pending = self.wait_queue.prepare_wait();
            drop(stdin);
            pending.enter_wait(WaitReason::CanonicalRead, None)?;

            return self.read_stdin(buf);
        }

        let max_read = if is_canonical {
            let first_newline = stdin.inner.iter().position(|c| *c == b'\n').unwrap();
            first_newline + 1
        } else {
            stdin.inner.len()
        };

        let read_len = buf.len().min(max_read);

        let mut newlines_ava = false;
        if is_canonical && read_len == max_read {
            stdin.newlines_count -= 1;
            newlines_ava = stdin.newlines_count > 0;
        }

        buf[..read_len].copy_from_slice(&stdin.inner[..read_len]);
        stdin.inner.copy_within(read_len.., 0);
        unsafe {
            let stdin_bytes_len = stdin.inner.len();
            stdin.inner.set_len(stdin_bytes_len - read_len);
        }

        if stdin.inner.len() == 0 || (is_canonical && !newlines_ava) {
            poll::broadcast_events(
                self.child_poll_id(),
                PollEvents::NONE,
                PollEvents::DATA_AVAILABLE,
            );
        }
        Ok(read_len)
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

    pub fn child_poll_id(&self) -> PollID {
        let ptr = self as *const _ as usize;
        // Self is size 0x70, so ptr + 1 is still within the bounds of the struct
        PollID::from_usize(ptr + 1)
    }

    pub fn mother_poll_id(&self) -> PollID {
        PollID::from_ptr(self)
    }
}

impl Drop for VirtualTTY {
    fn drop(&mut self) {
        poll::stop_tracking_id(self.mother_poll_id());
        poll::stop_tracking_id(self.child_poll_id());
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
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, ()> {
        self.tty.read_stdout(buf)
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
        // We wrote data to the mother
        poll::broadcast_events(
            self.tty.mother_poll_id(),
            PollEvents::DATA_AVAILABLE,
            PollEvents::NONE,
        );
        s.len()
    }
    /// Performs a read operation from stdin
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, WaitError> {
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

impl Resource for MotherVTTY {
    fn read(&self, off: SeekOffset, buf: &mut [u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        self.read(buf).map_err(|()| ErrorStatus::InvalidOffset)
    }

    fn write(&self, off: SeekOffset, buf: &[u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        let buf_str = str::from_utf8(buf)?;
        Ok(self.write(buf_str))
    }

    fn try_clone_into_node(
        &self,
    ) -> Result<crate::process::resources::ResourceNodeRef, ErrorStatus> {
        resources::generic_clone_impl(self)
    }

    fn sync(&self) -> Result<(), ErrorStatus> {
        // TODO: Implement Sync and buffering
        Ok(())
    }

    fn address_space_generic(&self) -> bool {
        true
    }

    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
        self.process_command(cmd, arg)
    }

    fn poll_id(&self) -> Option<PollID> {
        Some(self.tty.mother_poll_id())
    }
}

impl Resource for ChildVTTY {
    fn read(&self, off: SeekOffset, buf: &mut [u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        Ok(self.read(buf)?)
    }

    fn write(&self, off: SeekOffset, buf: &[u8]) -> Result<usize, ErrorStatus> {
        _ = off;
        let buf_str = str::from_utf8(buf)?;
        Ok(self.write(buf_str))
    }

    fn try_clone_into_node(
        &self,
    ) -> Result<crate::process::resources::ResourceNodeRef, ErrorStatus> {
        resources::generic_clone_impl(self)
    }

    fn sync(&self) -> Result<(), ErrorStatus> {
        // TODO: Implement Sync and buffering
        Ok(())
    }

    fn address_space_generic(&self) -> bool {
        true
    }

    fn send_command(&self, cmd: u16, arg: u64) -> Result<(), ErrorStatus> {
        self.process_command(cmd, arg)
    }

    fn poll_id(&self) -> Option<PollID> {
        Some(self.tty.child_poll_id())
    }
}

#[allow(unused_assignments)]
#[test_case]
fn vtty_canonical_mode_test() {
    use crate::thread;
    use core::sync::atomic::{AtomicBool, Ordering};
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

    macro_rules! assert_child {
        ($buf: ident, $c: ident, $o: expr) => {{
            let read_buf = &mut $buf;
            let child = &$c;
            let len = child.read(read_buf).expect("Failed to read");
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
                .read(&mut read_buf[..o.len()])
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

#[allow(unused_assignments)]
#[test_case]
fn vtty_non_canonical_mode_test() {
    const MSG0: &str = "Hello, world!\n";
    const SPEC_MSG: &str = "hi\x7flol\n";
    const SPEC_MSG_REPLY: &str = "hi\x7flol\n";
    const SPEC_MSG_REPLY_STDOUT0: &str = "hi\x7flol\n";
    const SPEC_MSG_REPLY_STDOUT1: &str = "hi\x7flol\n";
    const MSG1: &str = ":cvvv\x08\x082222";

    let mut read_buf = [0u8; 2048];
    let (mother, child) = alloc_vtty();

    mother.set_flags(TTYFlags::ECHO);
    child.write(MSG0);
    mother.write(MSG0);
    mother.write(SPEC_MSG);
    mother.set_flags(TTYFlags::ECHO | TTYFlags::ECHO_ERASE);
    mother.write(SPEC_MSG);
    mother.write(MSG1);
    // No echo
    mother.set_flags(TTYFlags::empty());
    mother.write(MSG1);

    macro_rules! assert_child {
        ($buf: ident, $c: ident, $o: expr) => {{
            let read_buf = &mut $buf;
            let child = &$c;
            let o: &[u8] = $o;

            let len_half0 = child
                .read(&mut read_buf[..o.len() / 2])
                .expect("Failed to read from child");
            let len_half1 = child
                .read(&mut read_buf[o.len() / 2..o.len()])
                .expect("Failed to read from child");
            let len = len_half0 + len_half1;
            let read = &read_buf[..len];
            unsafe {
                assert_eq!(
                    read,
                    $o,
                    "read: '{}', expected: '{}'; from child",
                    str::from_utf8_unchecked(read),
                    str::from_utf8_unchecked($o)
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
                .read(&mut read_buf[..o.len()])
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
}
