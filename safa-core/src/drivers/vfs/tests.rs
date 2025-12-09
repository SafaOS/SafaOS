use core::time::Duration;

use crate::{
    timer::{DurationFmt, SystemInstant},
    utils::{
        path::{Path, make_path},
        types::DriveName,
    },
};

use crate::{
    drivers::vfs::{FSError, FSObjectDescriptor, FileSystem, SeekOffset, VFS, ramfs::RamFS},
    utils::locks::RwLock,
};

use crate::test_log;

fn test_filesystem() -> impl FileSystem {
    RwLock::new(RamFS::create())
}

fn mount_test_filesystem(vfs: &mut VFS) {
    let start_mount_time = SystemInstant::now();
    vfs.mount(DriveName::new_const("test"), test_filesystem())
        .expect("failed to mount filesystem");
    let end_mount_elapsed = start_mount_time.elapsed();

    test_log!(
        "mounted filesystem in {}",
        DurationFmt::new(end_mount_elapsed),
    );
}

fn create_test_file<'a>(vfs: &mut VFS) -> Path<'a> {
    let path = make_path!("test", "test_file.txt");

    let start_create_time = SystemInstant::now();
    vfs.createfile(path).expect("failed to create file");
    let create_elapsed = start_create_time.elapsed();

    test_log!("created file in {}", DurationFmt::new(create_elapsed));
    path
}

fn create_test_directory<'a>(vfs: &mut VFS) -> Path<'a> {
    let path = make_path!("test", "test_directory");
    let start_create_time = SystemInstant::now();
    vfs.createdir(path).expect("failed to create directory");
    let create_elapsed = DurationFmt::new(start_create_time.elapsed());
    test_log!("created directory in {}", create_elapsed);
    path
}

#[test_case]
fn a_mount_filesystem() {
    let mut vfs = VFS::new();
    mount_test_filesystem(&mut vfs);
}

#[test_case]
fn b_invalid_path_tests() {
    let mut vfs = VFS::new();
    // ==== Invalid Drive =======
    assert_eq!(
        vfs.createdir(make_path!("fake", "smthsmth")),
        Err(FSError::FSLabelNotFound)
    );
    // ==== Invalid Path =======
    assert_eq!(
        vfs.createdir(make_path!("fake", "")),
        Err(FSError::InvalidPath)
    );
    // ==== Not Found =======
    mount_test_filesystem(&mut vfs);
    assert_eq!(
        vfs.createdir(make_path!("test", "fake/smthsmth")),
        Err(FSError::NotFound)
    );
}

#[test_case]
fn c_create_stuff() {
    let mut vfs = VFS::new();
    mount_test_filesystem(&mut vfs);
    // ==== Creating file =======
    let test_path = create_test_file(&mut vfs);
    // ==== Creating an existing file =======
    assert_eq!(vfs.createfile(test_path), Err(FSError::AlreadyExists));
    // ==== Create directory =======
    let test_dir_path = create_test_directory(&mut vfs);
    // ==== Creating an existing directory =======
    assert_eq!(vfs.createdir(test_dir_path), Err(FSError::AlreadyExists));
    // ==== Create file in directory =======
    let create_start_time = SystemInstant::now();

    let test_file_path = make_path!("test", "test_directory/test_file");
    vfs.createfile(test_file_path)
        .expect("failed to create file in directory");
    let create_elapsed = create_start_time.elapsed();

    test_log!(
        "created file in directory in {}",
        DurationFmt::new(create_elapsed),
    );
    // ==== Creating an existing file in directory =======
    assert_eq!(vfs.createfile(test_file_path), Err(FSError::AlreadyExists));
}

#[test_case]
fn d_create_benchmarks() {
    let mut fmt_buffer = heapless::String::<20>::new();
    use core::fmt::Write;
    macro_rules! path_to_test_file {
        ($n: expr_2021) => {{
            write!(&mut fmt_buffer, "test_file_{}", $n).expect("failed to generate test file name");
            let path = make_path!("test", &*fmt_buffer);
            path
        }};
    }

    let mut vfs = VFS::new();

    // ======= Mounting Filesystem =======
    mount_test_filesystem(&mut vfs);
    // ======= Creating Files =======
    const CREATE_AMOUNT: usize = 100;
    let mut results = heapless::Vec::<Duration, CREATE_AMOUNT>::new();
    for i in 0..CREATE_AMOUNT {
        let path = path_to_test_file!(i);
        let create_start_time = SystemInstant::now();
        // === actually create ====
        vfs.createfile(path).expect("failed to create file");
        let create_elapsed = create_start_time.elapsed();

        // cleanup
        fmt_buffer.clear();

        results.push(create_elapsed).unwrap();
    }

    fn calculate_results_time(
        results: &[Duration],
    ) -> (DurationFmt, DurationFmt, DurationFmt, DurationFmt) {
        let mut total_time = Duration::ZERO;
        let mut peak_time = Duration::ZERO;
        let mut min_time = Duration::MAX;

        for time in results.iter() {
            total_time += *time;
            if *time > peak_time {
                peak_time = *time;
            }
            if *time < min_time {
                min_time = *time;
            }
        }

        let average_time = total_time
            .checked_div(CREATE_AMOUNT as u32)
            .expect("Failed to calculate average time");
        (
            DurationFmt::new(total_time),
            DurationFmt::new(peak_time),
            DurationFmt::new(min_time),
            DurationFmt::new(average_time),
        )
    }

    macro_rules! log_results_time {
        ($results:expr_2021, $results_of: literal) => {{
            let (total_time, peak_time, min_time, average_time) =
                calculate_results_time(&*$results);
            test_log!(
                "'{}' {} files in {}, peak {}, min {}, average {}",
                $results_of,
                CREATE_AMOUNT,
                total_time,
                peak_time,
                min_time,
                average_time,
            );
        }};
    }

    log_results_time!(results, "created");

    // ====== Opening Files =======
    let mut results = heapless::Vec::<Duration, CREATE_AMOUNT>::new();
    let mut result_descriptors = heapless::Vec::<FSObjectDescriptor, CREATE_AMOUNT>::new();

    for i in 0..CREATE_AMOUNT {
        let path = path_to_test_file!(i);

        let open_instant = SystemInstant::now();
        // === actually open ====
        let descriptor = vfs.open_all(path).expect("failed to open file");
        let open_elapsed = open_instant.elapsed();

        //clean up
        fmt_buffer.clear();

        results.push(open_elapsed).unwrap();
        _ = result_descriptors.push(descriptor);
    }

    log_results_time!(results, "opened");

    // ===== Write to Files =====
    let mut results = heapless::Vec::<Duration, CREATE_AMOUNT>::new();
    const WRITE_MESSAGE: &[u8] = b"Hello, World!";

    for i in 0..CREATE_AMOUNT {
        let fd = &mut result_descriptors[i];
        let write_start_time = SystemInstant::now();
        // actually write to files
        fd.write(SeekOffset::Start(0), WRITE_MESSAGE)
            .expect("failed to write to file");
        let write_elapsed = write_start_time.elapsed();

        results.push(write_elapsed).unwrap();
    }

    log_results_time!(results, "wrote");

    // ===== Read from Files =====
    let mut results = heapless::Vec::<Duration, CREATE_AMOUNT>::new();

    for i in 0..CREATE_AMOUNT {
        let fd = &mut result_descriptors[i];
        let mut buf = [0; (WRITE_MESSAGE).len()];

        let read_start_time = SystemInstant::now();
        // actually read from files
        fd.read(SeekOffset::Start(0), &mut buf)
            .expect("failed to write to file");
        let read_elapsed = read_start_time.elapsed();

        // verify results
        assert_eq!(
            &buf, WRITE_MESSAGE,
            "file {i} yielded invalid data after read"
        );

        results.push(read_elapsed).unwrap();
    }

    log_results_time!(results, "read");
}
