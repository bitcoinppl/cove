use std::fmt::Display;

pub trait ResultExt<T, E> {
    /// Map an error to a string-based error variant
    ///
    /// This allows converting `Result<T, E>` to `Result<T, F>` where F has a variant
    /// that takes a String, using the Display implementation of E.
    ///
    /// # Example
    /// ```
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
    fn map_err_str<F>(self, f: fn(String) -> F) -> Result<T, F>
    where
        E: Display;

    /// map an error using Into::into before passing to error constructor
    ///
    /// shorthand for map_err(|e| f(e.into())) to convert errors using Into trait
    fn map_err_into<F, G>(self, f: fn(F) -> G) -> Result<T, G>
    where
        E: Into<F>;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn map_err_str<F>(self, f: fn(String) -> F) -> Result<T, F>
    where
        E: Display,
    {
        self.map_err(|e| f(e.to_string()))
    }

    fn map_err_into<F, G>(self, f: fn(F) -> G) -> Result<T, G>
    where
        E: Into<F>,
    {
        self.map_err(|e| f(e.into()))
    }
}
