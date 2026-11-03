/// Key-value storage that CSPP uses to persist its local master-key state
pub trait CsppStore: Send + Sync {
    type Error: std::fmt::Display;

    /// Saves `value` under `key`, replacing any existing value
    ///
    /// # Errors
    ///
    /// Returns an error when the value could not be persisted
    fn save(&self, key: String, value: String) -> Result<(), Self::Error>;

    /// Reads the value stored under `key`, or `None` when no value exists
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot tell whether `key` exists. Implementations must
    /// never report a read failure as `None`, since CSPP treats `None` as proof of absence
    fn get(&self, key: String) -> Result<Option<String>, Self::Error>;

    /// Deletes the value stored under `key`, returning whether the store reported success
    fn delete(&self, key: String) -> bool;
}
