//! Errors shared by metadata parsing, data reads and bindings.

use std::fmt;

/// A reader result with a structured failure.
pub type Result<T> = std::result::Result<T, Error>;

/// A file, feature, resource-limit or dataset-selection failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Io(std::io::Error),
    NotHdf4,
    Malformed {
        offset: u64,
        what: &'static str,
    },
    Unsupported {
        feature: &'static str,
        object: String,
    },
    LimitExceeded {
        limit: &'static str,
        value: u64,
        max: u64,
    },
    NotFound(String),
    Ambiguous {
        name: String,
        count: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::NotHdf4 => formatter.write_str("not an HDF4 file"),
            Self::Malformed { offset, what } => {
                write!(formatter, "malformed HDF4 at byte {offset}: {what}")
            }
            Self::Unsupported { feature, object } => {
                write!(formatter, "unsupported feature {feature} in {object}")
            }
            Self::LimitExceeded { limit, value, max } => {
                write!(formatter, "{limit} limit exceeded: {value} > {max}")
            }
            Self::NotFound(name) => write!(formatter, "dataset not found: {name}"),
            Self::Ambiguous { name, count } => {
                write!(formatter, "ambiguous dataset {name}: {count} matches")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn errors_explain_the_failure() {
        let cases = [
            (Error::NotHdf4, "not an HDF4 file"),
            (
                Error::Malformed {
                    offset: 12,
                    what: "truncated record",
                },
                "malformed HDF4 at byte 12: truncated record",
            ),
            (
                Error::Unsupported {
                    feature: "szip",
                    object: "dataset".into(),
                },
                "unsupported feature szip in dataset",
            ),
            (
                Error::LimitExceeded {
                    limit: "rank",
                    value: 33,
                    max: 32,
                },
                "rank limit exceeded: 33 > 32",
            ),
            (Error::NotFound("data".into()), "dataset not found: data"),
            (
                Error::Ambiguous {
                    name: "data".into(),
                    count: 2,
                },
                "ambiguous dataset data: 2 matches",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn io_error_preserves_its_source() {
        let error = Error::from(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "short read",
        ));
        assert_eq!(error.to_string(), "I/O error: short read");
        assert_eq!(
            error.source().expect("I/O source").to_string(),
            "short read"
        );
    }
}
