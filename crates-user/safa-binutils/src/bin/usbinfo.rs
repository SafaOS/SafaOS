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

        macro_rules! dprint_field {
            ($name: literal, $($args:tt)*) => (print_field!(1, 14, $name, $($args)*));
        }
        macro_rules! iprint_field {
            ($name: literal, $($args:tt)*) => (print_field!(2, 10, $name, $($args)*));
        }

        dprint_field!(
            "ID",
            "{:>04x}:{:>04x}",
            descriptor.id_vendor().yellow(),
            descriptor.id_product().yellow()
        );
        dprint_field!(
            "Class",
            "{:>04x}:{:>04x}:{:>04x}",
            descriptor.class().yellow(),
            descriptor.subclass().yellow(),
            descriptor.protocol().yellow()
        );

        dprint_field!("Manufacturer", "{}", manufacturer.bright_white());
        dprint_field!("Product", "{}", product.bright_white());
        dprint_field!("Serial Number", "{}", device.serial_number().red());

        for interface in device.interfaces() {
            let descriptor = interface.descriptor();
            dprint_field!("Interface", "");

            iprint_field!(
                "Class",
                "{:>04x}:{:>04x}",
                descriptor.class().yellow(),
                descriptor.subclass().yellow()
            );
            iprint_field!("Protocol", "{:>04x}", descriptor.protocol().yellow());

            let has_driver: &dyn std::fmt::Display = if interface.has_driver() {
                &"yes".bright_green()
            } else {
                &"no".bright_red()
            };

            iprint_field!("Has Driver", "{}", has_driver);
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
