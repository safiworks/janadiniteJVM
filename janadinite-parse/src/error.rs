extern crate alloc;
use core::fmt::{Debug, Display};

use alloc::boxed::Box;
use alloc::string::String;

/// A janadinite error, kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEof,
    InvalidData,
    Other,
}

impl ErrorKind {
    pub const fn explain(&self) -> &'static str {
        match self {
            Self::Other => "Unknown Error",
            Self::UnexpectedEof => "Unexpected End Of File",
            Self::InvalidData => "Corrupted",
        }
    }
}

/// A janadinite error with an explaintion.
#[derive(Clone, PartialEq, Eq)]
pub enum Error {
    Static(ErrorKind, &'static str),
    Allocated(ErrorKind, Box<str>),
}

impl Error {
    #[inline(always)]
    pub const fn new_simple(kind: ErrorKind) -> Self {
        Self::Static(kind, kind.explain())
    }

    #[inline(always)]
    pub const fn new_static(kind: ErrorKind, msg: &'static str) -> Self {
        Self::Static(kind, msg)
    }

    #[inline(always)]
    pub fn new(kind: ErrorKind, msg: impl Into<String>) -> Self {
        Self::Allocated(kind, msg.into().into_boxed_str())
    }

    #[inline(always)]
    pub const fn message(&self) -> &str {
        match self {
            Self::Allocated(_, m) => m,
            Self::Static(_, m) => m,
        }
    }

    #[inline(always)]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Allocated(k, _) | Self::Static(k, _) => *k,
        }
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(&self, f)?;
        f.write_str(" (")?;
        Debug::fmt(&self.kind(), f)?;
        f.write_str(")")?;

        Ok(())
    }
}
impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Static(k, "") => Debug::fmt(k, f),
            Self::Allocated(k, s) if s.is_empty() => Debug::fmt(k, f),
            Self::Static(_, s) => f.write_str(s),
            Self::Allocated(_, s) => f.write_str(s),
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(value: ErrorKind) -> Self {
        Self::new_static(value, value.explain())
    }
}

pub type Result<Ok> = core::result::Result<Ok, Error>;

#[cfg(feature = "std")]
mod std_impl {
    impl From<super::ErrorKind> for std::io::ErrorKind {
        fn from(value: super::ErrorKind) -> Self {
            match value {
                super::ErrorKind::Other => std::io::ErrorKind::Other,
                super::ErrorKind::InvalidData => std::io::ErrorKind::InvalidData,
                super::ErrorKind::UnexpectedEof => std::io::ErrorKind::UnexpectedEof,
            }
        }
    }

    impl From<std::io::ErrorKind> for super::ErrorKind {
        fn from(value: std::io::ErrorKind) -> Self {
            match value {
                std::io::ErrorKind::UnexpectedEof => super::ErrorKind::UnexpectedEof,
                std::io::ErrorKind::InvalidData => super::ErrorKind::InvalidData,
                _ => super::ErrorKind::Other,
            }
        }
    }

    impl From<super::Error> for std::io::Error {
        fn from(value: super::Error) -> Self {
            let kind = value.kind().into();
            std::io::Error::new(kind, value.message())
        }
    }

    impl From<std::io::Error> for super::Error {
        fn from(value: std::io::Error) -> super::Error {
            let k = value.kind().into();
            super::Error::new(k, value.to_string())
        }
    }

    impl std::error::Error for super::Error {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            None
        }
    }
}

#[macro_export]
macro_rules! invalid_data {
    ($msg:expr) => {
        $crate::io::Error::new_static($crate::io::ErrorKind::InvalidData, $msg)
    };
}

pub use invalid_data;
