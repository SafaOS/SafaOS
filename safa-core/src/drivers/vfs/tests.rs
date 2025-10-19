use crate::utils::{
    path::{Path, make_path},
    types::DriveName,
};

use crate::{
    drivers::vfs::{FSError, FSObjectDescriptor, FileSystem, SeekOffset, VFS, ramfs::RamFS},
    time,
    utils::locks::RwLock,
};

use crate::test_log;

fn test_filesystem() -> impl FileSystem {
    RwLock::new(RamFS::create())
}

fn mount_test_filesystem(vfs: &mut VFS) {
    let start_mount_time = time!(us);
    vfs.mount(DriveName::new_const("test"), test_filesystem())
        .expect("failed to mount filesystem");
    let end_mount_time = time!(us);

    test_log!(
        "mounted filesystem in {}us",
        end_mount_time - start_mount_time
    );
}

fn create_test_file<'a>(vfs: &mut VFS) -> Path<'a> {
    let path = make_path!("test", "test_file.txt");
    let start_create_time = time!(us);
    vfs.createfile(path).expect("failed to create file");
    let end_create_time = time!(us);
    test_log!("created file in {}us", end_create_time - start_create_time);
    path
}

fn create_test_directory<'a>(vfs: &mut VFS) -> Path<'a> {
    let path = make_path!("test", "test_directory");
    let start_create_time = time!(us);
    vfs.createdir(path).expect("failed to create directory");
    let end_create_time = time!(us);
    test_log!(
        "created directory in {}us",
        end_create_time - start_create_time
    );
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
    let create_start_time = time!(us);
    let test_file_path = make_path!("test", "test_directory/test_file");
    vfs.createfile(test_file_path)
        .expect("failed to create file in directory");
    let create_end_time = time!(us);
    test_log!(
        "created file in directory in {}us",
        create_end_time - create_start_time
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
    let mut results = heapless::Vec::<u64, CREATE_AMOUNT>::new();
    for i in 0..CREATE_AMOUNT {
        let path = path_to_test_file!(i);
        let create_start_time = time!(us);
        // === actually create ====
        vfs.createfile(path).expect("failed to create file");
        let create_end_time = time!(us);

        // cleanup
        fmt_buffer.clear();

        let delta_time = create_end_time - create_start_time;
        results.push(delta_time).unwrap();
    }

    fn calculate_results_time(results: &[u64]) -> (u64, u64, u64, u64) {
        let mut total_time = 0;
        let mut peak_time = 0;
        let mut min_time = u64::MAX;

        for time in results.iter() {
            total_time += time;
            if *time > peak_time {
                peak_time = *time;
            }
            if *time < min_time {
                min_time = *time;
            }
        }

        let average_time = total_time / CREATE_AMOUNT as u64;
        (total_time, peak_time, min_time, average_time)
    }

    macro_rules! log_results_time {
        ($results:expr_2021, $results_of: literal) => {{
            let (total_time, peak_time, min_time, average_time) = calculate_results_time(&*$results);
            test_log!(
                  "'{}' {} files in {}us ({}ms), peak {}us ({}ms), min {}us ({}ms), average {}us ({}ms)",
                  $results_of,
                  CREATE_AMOUNT,
                  total_time,
                  total_time / 1000,
                  peak_time,
                  peak_time / 1000,
                  min_time,
                  min_time / 1000,
                  average_time,
                  average_time / 1000
              );
        }};
    }

    log_results_time!(results, "created");

    // ====== Opening Files =======
    let mut results = heapless::Vec::<u64, CREATE_AMOUNT>::new();
    let mut result_descriptors = heapless::Vec::<FSObjectDescriptor, CREATE_AMOUNT>::new();

    for i in 0..CREATE_AMOUNT {
        let path = path_to_test_file!(i);
        let open_start_time = time!(us);
        // === actually open ====
        let descriptor = vfs.open_all(path).expect("failed to open file");
        let open_end_time = time!(us);

        //clean up
        fmt_buffer.clear();

        let delta_time = open_end_time - open_start_time;

        results.push(delta_time).unwrap();
        _ = result_descriptors.push(descriptor);
    }

    log_results_time!(results, "opened");

    // ===== Write to Files =====
    let mut results = heapless::Vec::<u64, CREATE_AMOUNT>::new();
    const WRITE_MESSAGE: &[u8] = b"Hello, World!";

    for i in 0..CREATE_AMOUNT {
        let fd = &mut result_descriptors[i];
        let write_start_time = time!(us);
        // actually write to files
        fd.write(SeekOffset::Start(0), WRITE_MESSAGE)
            .expect("failed to write to file");
        let write_end_time = time!(us);
        let delta_time = write_end_time - write_start_time;

        results.push(delta_time).unwrap();
    }

    log_results_time!(results, "wrote");

    // ===== Read from Files =====
    let mut results = heapless::Vec::<u64, CREATE_AMOUNT>::new();

    for i in 0..CREATE_AMOUNT {
        let fd = &mut result_descriptors[i];
        let mut buf = [0; (WRITE_MESSAGE).len()];

        let read_start_time = time!(us);
        // actually read from files
        fd.read(SeekOffset::Start(0), &mut buf)
            .expect("failed to write to file");
        let read_end_time = time!(us);

        // verify results
        assert_eq!(
            &buf, WRITE_MESSAGE,
            "file {i} yielded invalid data after read"
        );

        let delta_time = read_end_time - read_start_time;

        results.push(delta_time).unwrap();
    }

    log_results_time!(results, "read");
}
