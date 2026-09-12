#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod identifier;
mod model;
mod preprocess;

#[cfg(test)]
mod upstream_tests;

pub use identifier::{classify, default_identifier, rank, Identifier, LanguageScore, Options};
pub use model::Model;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidModel(String),
    InvalidConfiguration(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidModel(message) => write!(formatter, "invalid model: {message}"),
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid configuration: {message}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
