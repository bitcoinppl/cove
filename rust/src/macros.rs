#[macro_export]
macro_rules! impl_default_for {
    ($name:ident) => {
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}
