extern crate alloc;

use core::fmt::Display;

use alloc::{
    borrow::ToOwned,
    format,
    string::{String, ToString},
};

use super::errors::{ErrorStatus, IntoErr};

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
    InvaildPath,
    FailedToJoinPaths,
}

impl IntoErr for PathError {
    fn into_err(self) -> ErrorStatus {
        match self {
            Self::InvaildPath | Self::FailedToJoinPaths => ErrorStatus::InvaildPath,
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

    fn join(&self, other: Self) -> OwnedPathParts {
        let join = |parent: &str, child: &str| -> String {
            match (parent.is_empty(), child.is_empty()) {
                (true, true) => return String::new(),
                (true, false) => return child.to_owned(),
                (false, true) => return parent.to_owned(),
                (false, false) => (),
            }

            let parent = parent.trim_end_matches('/');
            let child = child.trim_start_matches('/');

            format!("{parent}/{child}")
        };

        let joined = join(self.inner, other.inner);
        OwnedPathParts { inner: joined }
    }

    fn to_owned(&self) -> OwnedPathParts {
        OwnedPathParts {
            inner: self.inner.to_owned(),
        }
    }

    #[inline(always)]
    pub fn new(inner: &'a str) -> Self {
        if inner.is_empty() {
            return Self::default();
        }

        let trimed = inner.trim();
        let trimed = trimed.trim_matches('/');

        assert!(!trimed.contains(':'));

        Self { inner: trimed }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPathParts {
    inner: String,
}

impl OwnedPathParts {
    pub fn as_path_parts(&self) -> PathParts<'_> {
        PathParts {
            inner: self.inner.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathBuf {
    drive: Option<String>,
    path: Option<OwnedPathParts>,
}

impl PathBuf {
    pub fn as_path(&self) -> Path<'_> {
        Path {
            drive: self.drive.as_deref(),
            path: self.path.as_ref().map(|x| x.as_path_parts()),
        }
    }
}

impl Display for PathBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(&self.as_path(), f)
    }
}

/// I failed to find a way to make this work with `Cow<'a, Path<'a>>` so I made this enum
#[derive(Debug, Clone, PartialEq)]
pub enum CowPath<'a> {
    Owned(PathBuf),
    Borrowed(Path<'a>),
}

impl<'a> CowPath<'a> {
    pub fn as_path(&'a self) -> Path<'a> {
        match self {
            Self::Owned(path) => path.as_path(),
            Self::Borrowed(path) => *path,
        }
    }

    pub fn into_owned(self) -> PathBuf {
        match self {
            Self::Owned(path) => path,
            Self::Borrowed(path) => path.into_owned(),
        }
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
    pub fn into_owned(self) -> PathBuf {
        PathBuf {
            drive: self.drive.map(|s| s.to_owned()),
            path: self.path.map(|x| x.to_owned()),
        }
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
        let thrid_part = parts.next();

        if thrid_part.is_some() {
            return Err(PathError::InvaildPath);
        }
        let (drive, path) = match (first_part, second_part) {
            // paths like `sys:whatever` are ugly `sys:/whatever` is the correct way
            (Some(drive), Some(path)) if path.starts_with('/') => (Some(drive), Some(path)),
            // relative paths must not start with a `/` i forgot why
            (Some(path), None) if !path.starts_with('/') => (None, Some(path)),
            (None, Some(_)) | (None, None) => unreachable!(),
            _ => return Err(PathError::InvaildPath),
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

    pub fn join(&self, other: Self) -> Result<PathBuf, PathError> {
        let drive = match (self.drive, other.drive) {
            (None, None) => None,
            (Some(drive), None) | (None, Some(drive)) => Some(drive),
            _ => return Err(PathError::FailedToJoinPaths),
        };

        let path = match (self.path, other.path) {
            (None, None) => None,
            (Some(path), None) | (None, Some(path)) => Some(path.to_owned()),
            (Some(path), Some(other_path)) => Some(path.join(other_path)),
        };

        Ok(PathBuf {
            drive: drive.map(|s| s.to_string()),
            path,
        })
    }

    #[inline(always)]
    pub fn is_absolute(&self) -> bool {
        self.drive.is_some()
    }

    /// converts the path to an absolute path if it is relative, the resulted path is going to be absolute to the results of `abs_other`
    pub fn to_absolute_with(self, abs_other: impl FnOnce() -> CowPath<'a>) -> CowPath<'a> {
        if self.is_absolute() {
            CowPath::Borrowed(self)
        } else {
            let abs_other = abs_other();
            let abs_other = abs_other.as_path();

            assert!(abs_other.is_absolute());

            let joined = unsafe { abs_other.join(self).unwrap_unchecked() };
            CowPath::Owned(joined)
        }
    }
}
