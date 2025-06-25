use safa_utils::{make_path, path::Path, types::DriveName};

use crate::{
    drivers::vfs::{ramfs::RamFS, FSError, FileSystem, VFS},
    time,
    utils::locks::RwLock,
};

use crate::test_log;

fn test_filesystem() -> impl FileSystem {
    RwLock::new(RamFS::new())
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
        Err(FSError::InvalidDrive)
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
        Err(FSError::NoSuchAFileOrDirectory)
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
