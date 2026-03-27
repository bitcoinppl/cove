use std::fmt::{Debug, Display};

pub trait ResultExt<T, E> {
    /// Map an error to a string-based error variant
    ///
    /// This allows converting `Result<T, InitialError>` to `Result<T, FinalError>` where `FinalError` has a variant
    /// that takes a String, using the Display implementation of `InitialError`.
    ///
    /// # Example
    /// ```rust
    /// use cove_util::result_ext::ResultExt;
    ///
    /// #[derive(Debug, thiserror::Error)]
    /// enum MyError {
    ///     #[error("io error: {0}")]
    ///     Io(String),
    /// }
    ///
    /// fn example() -> Result<(), MyError> {
    ///     std::fs::read_to_string("nonexistent.txt")
    ///         .map_err_str(MyError::Io)?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    /// Returns the error transformed by the provided function
    fn map_err_str<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError;

    /// Map an error using alternate Display (`{:#}`) to include the full error chain
    ///
    /// # Errors
    /// Returns the error transformed by the provided function
    fn map_err_display_alt<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError;

    /// Map an error using Debug formatting (`{:?}`)
    ///
    /// # Errors
    /// Returns the error transformed by the provided function
    fn map_err_debug<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Debug,
        F: FnOnce(String) -> FinalError;

    /// Map an error with a prefix string, producing `"prefix: error_message"`
    ///
    /// # Errors
    /// Returns the error transformed by the provided function
    fn map_err_prefix<FinalError, F>(self, prefix: &str, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError;

    /// Map an error using `Into::into` before passing to error constructor
    ///
    /// Shorthand for `map_err(|e| f(e.into()))` to convert errors using Into trait
    ///
    /// # Errors
    /// Returns the error transformed by the provided function
    fn map_err_into<I, FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Into<I>,
        F: FnOnce(I) -> FinalError;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn map_err_str<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError,
    {
        self.map_err(|e| f(e.to_string()))
    }

    fn map_err_display_alt<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError,
    {
        self.map_err(|e| f(format!("{e:#}")))
    }

    fn map_err_debug<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Debug,
        F: FnOnce(String) -> FinalError,
    {
        self.map_err(|e| f(format!("{e:?}")))
    }

    fn map_err_prefix<FinalError, F>(self, prefix: &str, f: F) -> Result<T, FinalError>
    where
        E: Display,
        F: FnOnce(String) -> FinalError,
    {
        self.map_err(|e| f(format!("{prefix}: {e}")))
    }

    fn map_err_into<I, FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        E: Into<I>,
        F: FnOnce(I) -> FinalError,
    {
        self.map_err(|e| f(e.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug, PartialEq)]
    enum TestError {
        Msg(String),
    }

    #[derive(Debug)]
    struct SourceError(&'static str);

    impl fmt::Display for SourceError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // alternate format includes extra context
            if f.alternate() { write!(f, "{}: detailed", self.0) } else { write!(f, "{}", self.0) }
        }
    }

    #[test]
    fn map_err_str_uses_display() {
        let result: Result<(), SourceError> = Err(SourceError("oops"));
        let mapped = result.map_err_str(TestError::Msg);
        assert_eq!(mapped.unwrap_err(), TestError::Msg("oops".into()));
    }

    #[test]
    fn map_err_display_alt_uses_alternate() {
        let result: Result<(), SourceError> = Err(SourceError("oops"));
        let mapped = result.map_err_display_alt(TestError::Msg);
        assert_eq!(mapped.unwrap_err(), TestError::Msg("oops: detailed".into()));
    }

    #[test]
    fn map_err_debug_uses_debug() {
        let result: Result<(), SourceError> = Err(SourceError("oops"));
        let mapped = result.map_err_debug(TestError::Msg);
        assert_eq!(mapped.unwrap_err(), TestError::Msg("SourceError(\"oops\")".into()));
    }

    #[test]
    fn map_err_prefix_prepends_prefix() {
        let result: Result<(), SourceError> = Err(SourceError("oops"));
        let mapped = result.map_err_prefix("loading config", TestError::Msg);
        assert_eq!(mapped.unwrap_err(), TestError::Msg("loading config: oops".into()));
    }
}
