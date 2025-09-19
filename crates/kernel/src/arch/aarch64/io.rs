#[allow(unused)]
pub unsafe fn inb(port: u16) -> u8 {
    _ = port;
    unimplemented!("inb isn't implemented for aarch64")
}
#[allow(unused)]
pub unsafe fn outb(port: u16, value: u8) {
    _ = port;
    _ = value;
    unimplemented!("outb isn't implemented for aarch64")
}

#[allow(unused)]
pub unsafe fn outw(port: u16, value: u16) {
    _ = port;
    _ = value;
    unimplemented!("outw isn't implemented for aarch64")
}

#[allow(unused)]
pub unsafe fn outl(port: u16, value: u32) {
    _ = port;
    _ = value;
    unimplemented!("outl isn't implemented for aarch64")
}

#[allow(unused)]
pub unsafe fn inw(port: u16) -> u16 {
    _ = port;
    unimplemented!("inw isn't implemented for aarch64")
}

#[allow(unused)]
pub unsafe fn inl(port: u16) -> u32 {
    unimplemented!("inl isn't implemented for aarch64")
}
