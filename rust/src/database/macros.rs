#[macro_export]
macro_rules! string_config_accessor {
    ($fn_name:ident, $key:expr, $return_type:ty, $update_variant:path) => {
        pub fn $fn_name(&self) -> Result<$return_type, Error> {
            use std::str::FromStr as _;

            let Some(value) = self
                .get($key)
                .map_err(|error| Error::DatabaseAccess(error.to_string()))?
            else {
                return Ok(Default::default());
            };

            let parsed = <$return_type>::from_str(&value).map_err(|_| {
                GlobalConfigTableError::Read(format!("unable to parse {}", stringify!($fn_name)))
            })?;

            Ok(parsed)
        }

        paste::paste! {
            pub fn [<set_ $fn_name>](&self, value: $return_type) -> Result<(), Error> {
                let value_to_send = if std::mem::needs_drop::<$return_type>() {
                    value.clone()
                } else {
                    value
                };

                let value = value.to_string();
                self.set($key, value)?;
                Updater::send_update($update_variant(value_to_send));

                Ok(())
            }
        }
    };
}
