extern crate alloc;
use core::fmt::{Display, Write};

use safa_abi::consts;

use crate::types::DriveName;

use super::errors::{ErrorStatus, IntoErr};

type RawPath = heapless::String<{ consts::MAX_PATH_LENGTH }>;

/// A macro to create a path
/// assumes that the given path is valid and therefore unchecked and unsafe
/// there are three variants of this macro
/// 1. make_path!() very safe to use creates an empty path
/// 2. make_path!($path:literal) unsafe if path contains empty parts, or a colon `:`
/// 3. make_path!($drive:literal, $path:expr) unsafe if path contains empty parts, and if the drive or path contains a colon `:`
/// [defined at crate::utils::path::make_path]
#[macro_export]
macro_rules! make_path {
    () => {
        use $crate::path::Path;
        Path::empty()
    };
    ($path:literal) => {
        unsafe {
            use $crate::path::{Path, PathParts};
            let parts = PathParts::new($path);
            Path::from_raw_parts(None, Some(parts))
        }
    };
    ($drive:literal, $path:expr) => {
        unsafe {
            use $crate::path::{Path, PathParts};
            // common mistake to put a colon at the end of the drive
            debug_assert!(!$drive.ends_with(':'));
            let parts = PathParts::new($path);
            Path::from_raw_parts(Some($drive), Some(parts))
        }
    };
}
#[derive(Debug, Clone, Copy)]
pub enum PathError {
    InvalidPath,
    FailedToJoinPaths,
    PathPartsTooLong,
    DriveNameTooLong,
}

impl IntoErr for PathError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::InvalidPath | Self::FailedToJoinPaths => ErrorStatus::InvalidPath,
            Self::DriveNameTooLong => ErrorStatus::NoSuchAFileOrDirectory,
            Self::PathPartsTooLong => ErrorStatus::StrTooLong,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PathParts<'a> {
    inner: &'a str,
}

impl Display for PathParts<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(self.inner, f)
    }
}

impl<'a> PathParts<'a> {
    /// an iterator over the path parts
    /// the iterator will split the path on `/` and return the parts
    /// would never return empty strings
    pub fn iter(&self) -> impl Iterator<Item = &'a str> {
        self.inner.split('/').filter(|x| !x.is_empty())
    }

    fn into_owned(&self) -> Result<OwnedPathParts, ()> {
        Ok(OwnedPathParts {
            inner: RawPath::try_from(self.inner)?,
        })
    }

    /// Returns an owned simplified version of `self`
    fn simplify(&self) -> Result<OwnedPathParts, ()> {
        let mut new = OwnedPathParts::default();
        new.append_simplified(*self)?;
        Ok(new)
    }

    #[inline(always)]
    pub fn new(inner: &'a str) -> Self {
        if inner.is_empty() {
            return Self::default();
        }

        let trimmed = inner.trim();
        let trimmed = trimmed.trim_matches('/');

        assert!(!trimmed.contains(':'));

        Self { inner: trimmed }
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    /// Spilts the path into the inner most child and the rest of the path
    pub fn spilt_into_name(self) -> (Option<&'a str>, PathParts<'a>) {
        let inner = self.inner.trim_matches('/');
        if inner.is_empty() || inner == "/" {
            return (None, self);
        }

        let name_position = inner.char_indices().rev().find_map(|(i, c)| {
            // since we are trimming the path first we can assume there is at least one char after `/`
            if c == '/' {
                Some(i + 1)
            } else {
                None
            }
        });

        let name_index = match name_position {
            // if there is absloultely no `/` in the path we return the whole path as the name
            None => return (Some(inner), PathParts::default()),
            Some(name_index) => name_index,
        };

        let (path, name) = inner.split_at(name_index);

        (Some(name), PathParts::new(path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnedPathParts {
    inner: RawPath,
}

impl OwnedPathParts {
    pub fn as_path_parts(&self) -> PathParts<'_> {
        PathParts {
            inner: self.inner.as_str(),
        }
    }

    fn append(&mut self, other: PathParts) -> Result<(), ()> {
        match (self.inner.is_empty(), other.inner.is_empty()) {
            (true, true) => return Ok(()),
            (true, false) => return Ok(*self = other.into_owned()?),
            (false, true) => return Ok(()),
            (false, false) => (),
        }

        self.append_str(other.inner)?;
        Ok(())
    }

    /// Appends a simplified version of `other` to self
    fn append_simplified(&mut self, other: PathParts) -> Result<(), ()> {
        match (self.inner.is_empty(), other.inner.is_empty()) {
            (true, true) | (false, true) => return Ok(()),
            (false, false) | (true, false) => (),
        }

        for part in other.iter() {
            if part == "." {
                continue;
            }

            if part == ".." {
                self.remove_last_part();
            } else {
                self.append_str(part).unwrap();
            }
        }

        Ok(())
    }

    fn append_str(&mut self, other: &str) -> Result<(), ()> {
        let other = other.trim_start_matches('/');
        if other.is_empty() || other == "/" {
            return Ok(());
        }

        if !self.inner.ends_with('/') && self.inner != "/" && !self.inner.is_empty() {
            self.inner.write_char('/').map_err(|_| ())?;
        }
        self.inner.write_str(other).map_err(|_| ())?;
        Ok(())
    }

    fn remove_last_part(&mut self) {
        let inner = self.inner.trim_end_matches('/');
        if inner.is_empty() || inner == "/" {
            return;
        }

        let last_item_position = inner
            .char_indices()
            .rev()
            .find_map(|(i, c)| {
                // since we are trimming the path first we can assume there is at least one char after `/`
                if c == '/' {
                    Some(i + 1)
                } else {
                    None
                }
            })
            .unwrap_or_default();

        for _ in last_item_position..self.inner.len() {
            self.inner.pop();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathBuf {
    drive: Option<DriveName>,
    path: Option<OwnedPathParts>,
}

impl PathBuf {
    pub fn as_path(&self) -> Path<'_> {
        Path {
            drive: self.drive.as_deref().map(|v| &**v),
            path: self.path.as_ref().map(|x| x.as_path_parts()),
        }
    }
    pub fn append(&mut self, other: Path) -> Result<(), PathError> {
        match (&self.drive, other.drive) {
            (None, None) | (Some(_), None) => (),
            (None, Some(drive)) => {
                let drive = DriveName::try_from(drive).map_err(|()| PathError::DriveNameTooLong)?;
                self.drive = Some(drive);
            }
            (Some(expected), Some(got)) => {
                if expected.as_str() != got {
                    return Err(PathError::FailedToJoinPaths);
                }
            }
        };

        if let Some(other) = other.path {
            self.path
                .get_or_insert_default()
                .append(other)
                .map_err(|_| PathError::PathPartsTooLong)?;
        }
        Ok(())
    }

    /// Appends a simplified version of `other` to self
    pub fn append_simplified(&mut self, other: Path) -> Result<(), PathError> {
        match (&self.drive, other.drive) {
            (None, None) | (Some(_), None) => (),
            (None, Some(drive)) => {
                let drive = DriveName::try_from(drive).map_err(|()| PathError::DriveNameTooLong)?;
                self.drive = Some(drive);
            }
            (Some(expected), Some(got)) => {
                if expected.as_str() != got {
                    return Err(PathError::FailedToJoinPaths);
                }
            }
        };

        if let Some(other) = other.path {
            self.path
                .get_or_insert_default()
                .append_simplified(other)
                .map_err(|_| PathError::PathPartsTooLong)?;
        }
        Ok(())
    }
}

impl Display for PathBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(&self.as_path(), f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Path<'a> {
    drive: Option<&'a str>,
    path: Option<PathParts<'a>>,
}

impl Display for Path<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(drive) = self.drive() {
            write!(f, "{}:/", drive)?;
        }

        if let Some(path) = self.parts() {
            write!(f, "{}", path)?;
        }
        Ok(())
    }
}

impl<'a> Path<'a> {
    #[inline(always)]
    pub const unsafe fn from_raw_parts(
        drive: Option<&'a str>,
        path: Option<PathParts<'a>>,
    ) -> Self {
        Self { drive, path }
    }
    #[inline(always)]
    pub fn into_owned(self) -> Result<PathBuf, PathError> {
        let drive = self
            .drive
            .map(DriveName::try_from)
            .transpose()
            .map_err(|()| PathError::DriveNameTooLong)?;

        let path = self
            .path
            .map(|p| PathParts::into_owned(&p))
            .transpose()
            .map_err(|()| PathError::PathPartsTooLong)?;
        Ok(PathBuf { drive, path })
    }

    // Returns an Owned simplified version of `self`
    #[inline(always)]
    pub fn into_owned_simple(self) -> Result<PathBuf, PathError> {
        let drive = self
            .drive
            .map(DriveName::try_from)
            .transpose()
            .map_err(|()| PathError::DriveNameTooLong)?;

        let path = self
            .path
            .map(|p| PathParts::simplify(&p))
            .transpose()
            .map_err(|()| PathError::PathPartsTooLong)?;
        Ok(PathBuf { drive, path })
    }

    pub const fn empty() -> Self {
        Self {
            drive: None,
            path: None,
        }
    }

    #[inline]
    pub fn new(path: &'a str) -> Result<Self, PathError> {
        let path = path.trim();

        if path.is_empty() {
            return Ok(Self::empty());
        }
        // if the path ends with a `:` it is a drive duh
        if path.ends_with(':') && path.len() > 1 {
            let drive = Some(&path[..path.len() - 1]);
            return Ok(Self { drive, path: None });
        }

        let mut parts = path.split(':');

        let first_part = parts.next();
        let second_part = parts.next();
        let third_part = parts.next();

        if third_part.is_some() {
            return Err(PathError::InvalidPath);
        }
        let (drive, path) = match (first_part, second_part) {
            // paths like `sys:whatever` are ugly `sys:/whatever` is the correct way
            (Some(drive), Some(path)) if path.starts_with('/') => (Some(drive), Some(path)),
            // relative paths must not start with a `/` i forgot why
            (Some(path), None) if !path.starts_with('/') => (None, Some(path)),
            (None, Some(_)) | (None, None) => unreachable!(),
            _ => return Err(PathError::InvalidPath),
        };

        let parts = if let Some(path) = path {
            Some(PathParts::new(path))
        } else {
            None
        };

        Ok(Self { drive, path: parts })
    }

    #[inline]
    pub unsafe fn new_unchecked(path: &'a str) -> Self {
        unsafe { Self::new(path).unwrap_unchecked() }
    }

    pub fn parts(&self) -> Option<PathParts<'a>> {
        self.path
    }

    pub fn drive(&self) -> Option<&'a str> {
        self.drive
    }

    #[inline(always)]
    pub fn is_absolute(&self) -> bool {
        self.drive.is_some()
    }

    #[inline]
    /// Spilts the path into the inner most child and the rest of the path
    pub fn spilt_into_name(self) -> (Option<&'a str>, Self) {
        let (name, parts) = self.parts().unwrap_or_default().spilt_into_name();
        (name, unsafe {
            Path::from_raw_parts(self.drive, Some(parts))
        })
    }

    /// Returns the length of `self` as a formatted str
    pub fn len(&self) -> usize {
        let drive = self.drive.map(|s| s.len()).unwrap_or_default();
        let parts = self.path.map(|s| s.inner.len()).unwrap_or_default();

        drive + 2 /* :/ */ + parts
    }
}
