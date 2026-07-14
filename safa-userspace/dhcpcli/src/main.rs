use std::{io, path::Path};

use getrandom::Error;
use safa_api::errors::SysResult;

use crate::{dhcp::DHCPClient, nic::Nic};

mod dhcp;
mod nic;
mod packet;

macro_rules! tri_io {
    ($expr: expr) => {
        $crate::tri!($expr.map_err(|e| safa_api::errors::err_from_io_error_kind(e.kind())))
    };
}

#[macro_export]
macro_rules! tri {
    ($expr: expr) => {
        match $expr {
            Ok(data) => data,
            Err(e) => return safa_api::errors::SysResult::err(e),
        }
    };
}

fn my_entropy_source(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    let get_byte = || {
        let elapsed = std::time::UNIX_EPOCH
            .elapsed()
            .expect("Failed to get elapsed time");
        let byte = (elapsed.as_millis() + elapsed.as_secs() as u128) as u8;
        byte
    };

    for byte in buf {
        *byte = get_byte();
    }
    Ok(())
}

#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), Error> {
    let buf = unsafe {
        // fill the buffer with zeros
        core::ptr::write_bytes(dest, 0, len);
        // create mutable byte slice
        core::slice::from_raw_parts_mut(dest, len)
    };
    my_entropy_source(buf)
}

macro_rules! error {
    ($($arg:tt)*) => {{ println!("[ERROR]: {}", format_args!($($arg)*)) }};
}

fn run(path: &Path) -> io::Result<()> {
    println!("Using the NIC at: {}", path.display());
    let nic = Nic::open(path).expect("Failed to open nic device file");
    let mut client = DHCPClient::create(&nic)?;

    let offer = match client.discover() {
        Ok(o) => o,
        Err(e) => {
            error!("While performing DISCOVER, {e}");
            return Err(e.into());
        }
    };

    let our_ip = offer.our_addr;
    let server_ip = offer.server_addr;
    let accept_from = offer.offered_from;
    match client.request(our_ip, server_ip, accept_from) {
        Ok(()) => {}
        Err(e) => {
            error!("While performing REQUEST, {e}");
            return Err(e.into());
        }
    }

    println!("Configuring the NIC to use the offer: {offer:#?}");
    nic.configure_with_offer(&offer)?;
    println!("Success");
    Ok(())
}

fn main() -> SysResult {
    let mut args = std::env::args();
    let _program_name = args.next().expect("Expected program name");
    let path_string = args.next().expect("Expected NIC path to configure");
    let path = Path::new(&path_string);

    tri_io!(run(path));
    SysResult::ok(0)
}
