pub trait CsppStore: Send + Sync {
    type Error: std::fmt::Display;
    fn save(&self, key: String, value: String) -> Result<(), Self::Error>;
    fn get(&self, key: String) -> Option<String>;
    fn delete(&self, key: String) -> bool;
}
