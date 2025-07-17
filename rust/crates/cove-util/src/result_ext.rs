use std::fmt::Display;

pub trait ResultExt<T, InitialError> {
    /// Map an error to a string-based error variant
    ///
    /// This allows converting `Result<T, InitialError>` to `Result<T, FinalError>` where FinalError has a variant
    /// that takes a String, using the Display implementation of InitialError.
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
    fn map_err_str<FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        InitialError: Display,
        F: FnOnce(String) -> FinalError;

    /// map an error using Into::into before passing to error constructor
    ///
    /// shorthand for map_err(|e| f(e.into())) to convert errors using Into trait
    fn map_err_into<I, FinalError, F>(self, f: F) -> Result<T, FinalError>
    where
        InitialError: Into<I>,
        F: FnOnce(I) -> FinalError;
}

impl<Type, InitialError> ResultExt<Type, InitialError> for Result<Type, InitialError> {
    fn map_err_str<FinalError, F>(self, f: F) -> Result<Type, FinalError>
    where
        InitialError: Display,
        F: FnOnce(String) -> FinalError,
    {
        self.map_err(|e| f(e.to_string()))
    }

    fn map_err_into<I, FinalError, F>(self, f: F) -> Result<Type, FinalError>
    where
        InitialError: Into<I>,
        F: FnOnce(I) -> FinalError,
    {
        self.map_err(|e| f(e.into()))
    }
}
