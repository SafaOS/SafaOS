use std::io;

use extra::{json::USBInfo, tri_io};
use owo_colors::OwoColorize;
use safa_api::errors::SysResult;

const IDENT_WIDTH: usize = 2;

macro_rules! print_field {
    ($depth: expr, $min_width: expr, $name: literal, $($args:tt)*) => {
        for _ in 0..($depth * IDENT_WIDTH) {
            print!("  ");
        }
        println!("{:<width$}: {}", $name.bright_cyan(), format_args!($($args)*), width = $min_width);
    };
}
pub fn usbinfo() -> io::Result<()> {
    let usbinfo = USBInfo::fetch()?;
    for device in usbinfo.connected_devices() {
        let slot_id = device.slot_id();
        let product = device.product();
        let manufacturer = device.manufacturer();
        let descriptor = device.descriptor();

        print_field!(0, 6, "Device", "{}", slot_id.yellow());
        print_field!(
            1,
            13,
            "ID",
            "{:>04x}:{:>04x}",
            descriptor.id_vendor().yellow(),
            descriptor.id_product().yellow()
        );
        print_field!(
            1,
            13,
            "Class",
            "{:>04x}:{:>04x}:{:>04x}",
            descriptor.class().yellow(),
            descriptor.subclass().yellow(),
            descriptor.protocol().yellow()
        );
        print_field!(1, 13, "Manufacturer", "{}", manufacturer.bright_white());
        print_field!(1, 13, "Product", "{}", product.bright_white());
        print_field!(1, 13, "Serial Number", "{}", device.serial_number().red());

        for interface in device.interfaces() {
            let descriptor = interface.descriptor();
            print_field!(1, 10, "Interface", "");

            print_field!(
                2,
                10,
                "Class",
                "{:>04x}:{:>04x}",
                descriptor.class().yellow(),
                descriptor.subclass().yellow()
            );
            print_field!(2, 10, "Protocol", "{:>04x}", descriptor.protocol().yellow());

            let has_driver: &dyn std::fmt::Display = if interface.has_driver() {
                &"yes".bright_green()
            } else {
                &"no".bright_red()
            };
            print_field!(2, 10, "Has Driver", "{}", has_driver);
        }
    }
    Ok(())
}
pub fn main() -> SysResult {
    if let Err(err) = usbinfo() {
        let mut args = std::env::args();

        let program = args.next();
        let program = program.as_ref().map(|s| &**s).unwrap_or("usbinfo");

        eprintln!("{program}: error {err}");

        tri_io!(Err(err));
    }
    SysResult::Success
}
