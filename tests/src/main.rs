use std::{
    borrow::Cow,
    cell::UnsafeCell,
    fmt::Debug,
    fs::{self, OpenOptions},
    mem::MaybeUninit,
    panic::PanicHookInfo,
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
};

const TEST_LOG_PATH: &str = "ram:/test.log";

macro_rules! log {
    ($($arg:tt)*) => {
        println!("\x1b[36m[TEST]\x1b[0m: {}", format_args!($($arg)*));
    };
}

macro_rules! log_fail {
    ($($arg:tt)*) => {
        println!("\x1b[31m[FAILED]: {}\x1b[0m", format_args!($($arg)*));
    };
}

use safa_api::errors::ErrorStatus;

fn panic_hook(info: &PanicHookInfo) {
    log_fail!("{info}");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OSError {
    Err(ErrorStatus),
    Other(u32),
}
impl OSError {
    fn from_exit_status(status: ExitStatus) -> Result<(), Self> {
        if status.success() {
            return Ok(());
        }

        let code = status.code().unwrap() as u32;
        if code > u16::MAX as u32 {
            return Err(OSError::Other(code));
        }

        Err(match ErrorStatus::try_from(code as u16) {
            Ok(err) => OSError::Err(err),
            Err(()) => OSError::Other(code),
        })
    }
}

struct Output {
    stdout: Cow<'static, str>,
    result: Result<(), OSError>,
}

impl Output {
    fn create_static(stdout: &'static str) -> Self {
        Self {
            stdout: Cow::Borrowed(stdout),
            result: Ok(()),
        }
    }

    fn is_success(&self) -> bool {
        self.result.is_ok()
    }

    fn stdout(&self) -> &str {
        &self.stdout
    }
}

impl Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "exit with output {:?}", &self.result)?;
        writeln!(f, "stdout({}):\n{}", self.stdout.len(), &self.stdout)?;
        Ok(())
    }
}

fn execute_binary(path: &'static str, args: &[&str]) -> Output {
    let stdout_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(TEST_LOG_PATH)
        .expect(&format!(
            "failed to open stdout file located at {}",
            TEST_LOG_PATH
        ));
    let stderr_file = stdout_file.try_clone().unwrap();

    let output = Command::new(path)
        .args(args)
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file))
        .output()
        .expect("failed to execute a binary");

    let (status, stdout) = (output.status, output.stdout);
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let result = OSError::from_exit_status(status);

    Output {
        stdout: Cow::Owned(stdout),
        result,
    }
}

fn do_test(path: &'static str, args: &[&str], expected_output: Output) {
    let output = execute_binary(path, args);

    let reason = match (
        output.result == expected_output.result,
        output.stdout() == expected_output.stdout(),
    ) {
        (true, true) => return,
        (false, false) => "Unexpected result and stdout",
        (true, false) => "Unexpected stdout",
        (false, true) => "Unexpected result",
    };

    panic!(
        "got: {output:#?}\nexpected: {expected_output:#?}\nREASON: {}",
        reason
    )
}

enum TestInner {
    Typical {
        path: &'static str,
        args: &'static [&'static str],
        expected_stdout: &'static str,
    },
    Special(fn()),
}

struct Test {
    name: &'static str,
    inner: TestInner,
}

impl Test {
    const fn new(
        name: &'static str,
        path: &'static str,
        args: &'static [&'static str],
        expected_stdout: &'static str,
    ) -> Self {
        Self {
            name,
            inner: TestInner::Typical {
                path,
                args,
                expected_stdout,
            },
        }
    }

    const fn new_special(name: &'static str, f: fn()) -> Self {
        Self {
            name,
            inner: TestInner::Special(f),
        }
    }

    fn execute(&self) {
        log!("Running test \"{}\"", self.name);
        match self.inner {
            TestInner::Typical {
                path,
                args,
                expected_stdout,
            } => do_test(path, args, Output::create_static(expected_stdout)),
            TestInner::Special(f) => f(),
        }

        println!("\x1b[32m[OK]\x1b[0m");
    }
}

const TEST_LIST: &[Test] = &[
    Test::new("Writing to Stdout", "sys:/bin/echo", &["hello"], "hello\n"),
    Test::new_special("[startup info capture]", || unsafe {
        let output = execute_binary("sys:/bin/meminfo", &["-k", "-r"]);
        MEMORY_INFO_CAPTURE.put(output);
        // Don't assert for success because if it fails the reason why it failed might be more visible from later tests
    }),
    Test::new("Creating Directories", "sys:/bin/mkdir", &["test"], ""),
    Test::new("Creating Files", "sys:/bin/touch", &["test/test_file"], ""),
    Test::new(
        "Writing to Created File",
        "sys:/bin/write",
        &["test/test_file", "test data"],
        "",
    ),
    Test::new(
        "Reading from Created File",
        "sys:/bin/cat",
        &["test/test_file"],
        "test data\n",
    ),
    Test::new_special("Chdir", || {
        std::env::set_current_dir("test").expect("failed to chdir");
        assert_eq!(std::env::current_dir().unwrap(), PathBuf::from("ram:/test"))
    }),
    Test::new_special("List Files", || {
        let read_dir = fs::read_dir(".").expect("failed to open the current directory");
        let mut file_list = Vec::with_capacity(2);

        for file in read_dir {
            let file = file.unwrap();
            file_list.push(file.file_name());
        }

        let file_list = file_list
            .iter()
            .map(|o| o.to_str().unwrap())
            .collect::<Vec<_>>();

        assert!(file_list.contains(&".."));
        assert!(file_list.contains(&"test_file"));
    }),
    Test::new_special("[memory leak detection]", || {
        let output = execute_binary("sys:/bin/meminfo", &["-k", "-r"]);
        let expected = unsafe { MEMORY_INFO_CAPTURE.get() };

        assert!(
            output.is_success() && expected.is_success(),
            "Either expected output: {:?} or retrived output {:?} or both results are a failure",
            expected.result,
            output.result
        );

        if output.stdout() != expected.stdout() {
            log_fail!(
                "Possible Memory Leak Detected\nexpected (`meminfo -k` output):\n{}\nbut got:\n{}",
                expected.stdout().trim_end_matches('\n'),
                output.stdout().trim_end_matches('\n'),
            );
        }
    }),
];

struct MemoryInfoCapture(UnsafeCell<MaybeUninit<Output>>);
impl MemoryInfoCapture {
    const fn new() -> Self {
        Self(UnsafeCell::new(MaybeUninit::uninit()))
    }

    unsafe fn put(&'static self, output: Output) {
        unsafe {
            *self.0.get() = MaybeUninit::new(output);
        }
    }

    unsafe fn get(&'static self) -> &'static Output {
        unsafe {
            let got = &*self.0.get();
            got.assume_init_ref()
        }
    }
}

unsafe impl Sync for MemoryInfoCapture {}
unsafe impl Send for MemoryInfoCapture {}

// Capturing MemoryInfo at startup to detect memory leaks
// Capture happens in the second test, (makes sure process spawning is working before capture)
static MEMORY_INFO_CAPTURE: MemoryInfoCapture = MemoryInfoCapture::new();

fn main() {
    // makes sure panic uses the custom println
    std::panic::set_hook(Box::new(panic_hook));
    log!("Running {} tests", TEST_LIST.len());

    for test in TEST_LIST {
        test.execute();
    }

    log!("Done running all tests");
}
