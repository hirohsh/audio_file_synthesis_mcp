use std::fmt::{Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum AppError {
    InvalidParams(String),
    UnsupportedFormat(String),
    Decode(String),
    Format(String),
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },
}

impl AppError {
    pub fn io_with_path(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: Some(path.to_path_buf()),
            source,
        }
    }
}

impl Display for AppError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(message) => write!(f, "invalid params: {message}"),
            Self::UnsupportedFormat(message) => write!(f, "unsupported format: {message}"),
            Self::Decode(message) => write!(f, "decode error: {message}"),
            Self::Format(message) => write!(f, "format error: {message}"),
            Self::Io {
                path: Some(path),
                source,
            } => write!(f, "io error at {}: {source}", path.display()),
            Self::Io { path: None, source } => write!(f, "io error: {source}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for AppError {
    fn from(source: io::Error) -> Self {
        Self::Io { path: None, source }
    }
}
