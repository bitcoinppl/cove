//! Payload-free JSON parsing errors
//!
//! serde_json's error text quotes the rejected value, for example `invalid value: string
//! "..."`. Callers that parse decrypted or user-supplied JSON convert the error to a
//! [`JsonDecodeError`] before logging or returning it, so only the failure category and its
//! position survive

use std::fmt;

/// What kind of JSON failure happened, without any of the input
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JsonErrorCategory {
    Io,
    Syntax,
    Data,
    EndOfInput,
}

impl fmt::Display for JsonErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Io => "I/O",
            Self::Syntax => "syntax",
            Self::Data => "data",
            Self::EndOfInput => "end of input",
        };

        f.write_str(name)
    }
}

/// Where in the JSON text a failure happened, 1-based
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsonPosition {
    pub line: usize,
    pub column: usize,
}

/// A JSON parse failure reduced to its category and position
///
/// Carries no data from the input, so its `Display` is safe to log and show
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub struct JsonDecodeError {
    pub category: JsonErrorCategory,

    /// `None` for errors from `serde_json::from_value`, which have no source text
    pub position: Option<JsonPosition>,
}

impl fmt::Display for JsonDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(JsonPosition { line, column }) => {
                write!(f, "{} error at line {line} column {column}", self.category)
            }
            None => write!(f, "{} error", self.category),
        }
    }
}

impl From<&serde_json::Error> for JsonDecodeError {
    fn from(error: &serde_json::Error) -> Self {
        use serde_json::error::Category;

        let category = match error.classify() {
            Category::Io => JsonErrorCategory::Io,
            Category::Syntax => JsonErrorCategory::Syntax,
            Category::Data => JsonErrorCategory::Data,
            Category::Eof => JsonErrorCategory::EndOfInput,
        };

        // serde_json reports line 0 when the error has no source position
        let position = match (error.line(), error.column()) {
            (0, _) => None,
            (line, column) => Some(JsonPosition { line, column }),
        };

        Self { category, position }
    }
}

impl From<serde_json::Error> for JsonDecodeError {
    fn from(error: serde_json::Error) -> Self {
        Self::from(&error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_values_stay_out_of_the_message() {
        let marker = "private-user-text";
        let from_text = serde_json::from_str::<u8>(&format!("\"{marker}\"")).unwrap_err();
        let from_value = serde_json::from_value::<u8>(serde_json::json!(marker)).unwrap_err();

        // serde's own text quotes the value, which is why this type exists
        assert!(from_text.to_string().contains(marker));
        assert!(from_value.to_string().contains(marker));

        assert_eq!(JsonDecodeError::from(&from_text).to_string(), "data error at line 1 column 19");
        assert_eq!(JsonDecodeError::from(&from_value).to_string(), "data error");
    }
}
