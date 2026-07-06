use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    InvalidArgument(String),
    Corruption(String),
    Io(std::io::Error),
    Closed,
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::Corruption(msg) => write!(f, "corruption: {msg}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Closed => write!(f, "database is closed"),
            Self::Unsupported(feature) => write!(f, "unsupported feature: {feature}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
