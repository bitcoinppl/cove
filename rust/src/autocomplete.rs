use crate::impl_default_for;

#[uniffi::export(with_foreign)]
pub trait AutoComplete: Send + Sync + std::fmt::Debug + 'static {
    fn autocomplete(&self, word: String) -> Vec<String>;
}

#[derive(Debug, Copy, Clone, uniffi::Object)]
pub struct Bip39AutoComplete;

impl_default_for!(Bip39AutoComplete);

#[uniffi::export]
impl Bip39AutoComplete {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }
}

#[uniffi::export]
impl AutoComplete for Bip39AutoComplete {
    fn autocomplete(&self, word: String) -> Vec<String> {
        vec![]
    }
}
