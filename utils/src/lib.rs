#![no_std]
pub mod ansi;
pub mod bstr;
pub mod display;
pub mod either;
pub use safa_abi as abi;
pub use safa_abi::consts;
pub use safa_abi::errors;
pub mod io;
pub mod path;
pub mod syscalls;

pub mod types {
    use core::{borrow::Borrow, ops::Deref};

    use safa_abi::consts;

    pub type Name = heapless::String<{ consts::MAX_NAME_LENGTH }>;

    #[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
    /// Wrapper around [heapless::String<N>] that provides additional functionality
    pub struct HeaplessString<const N: usize>(heapless::String<N>);

    impl<const N: usize> Borrow<str> for HeaplessString<N> {
        fn borrow(&self) -> &str {
            &self.0
        }
    }

    impl<const N: usize> Deref for HeaplessString<N> {
        type Target = heapless::String<N>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<const N: usize> HeaplessString<N> {
        #[inline(always)]
        /// Creates a new [`HeaplessString<N>`] from a static str, panics if length is more then `N`
        pub fn new_const(str: &'static str) -> Self {
            let inner =
                heapless::String::try_from(str).expect("HeaplessString::new_const: str too long");
            Self(inner)
        }
    }

    impl<const N: usize> From<heapless::String<N>> for HeaplessString<N> {
        #[inline(always)]
        fn from(value: heapless::String<N>) -> Self {
            Self(value)
        }
    }

    impl<'a, const N: usize> TryFrom<&'a str> for HeaplessString<N> {
        type Error = <heapless::String<N> as TryFrom<&'a str>>::Error;
        fn try_from(value: &'a str) -> Result<Self, Self::Error> {
            Ok(heapless::String::try_from(value)?.into())
        }
    }

    pub type DriveName = HeaplessString<{ consts::MAX_DRIVE_NAME_LENGTH }>;
    pub type FileName = HeaplessString<{ consts::MAX_NAME_LENGTH }>;
}
