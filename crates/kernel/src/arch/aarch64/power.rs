#![allow(dead_code)]

pub fn shutdown() -> ! {
    loop {}
}

pub fn reboot() -> ! {
    unreachable!("failed to reboot")
}
