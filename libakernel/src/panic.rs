use crate::logging::FFIArguments;

unsafe extern "C" {
    pub fn Safa_kpanic(msg: FFIArguments) -> !;
}

#[cfg(not(any(test, feature = "custom_panic_handler")))]
mod _inner {
    use super::*;
    use crate::logging::FFIArguments;
    use core::panic::PanicInfo;

    #[panic_handler]
    fn panic(info: &PanicInfo) -> ! {
        unsafe { Safa_kpanic(FFIArguments::new(&format_args!("{info}"))) }
    }
}
