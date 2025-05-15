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

#[macro_export]
macro_rules! new_type {
    ($name:ident, String) => {
        new_type!($name, String, std::any::type_name::<$name>());
    };

    ($name:ident, String, $redb_type_name:expr) => {
        uniffi::custom_newtype!($name, String);

        #[derive(
            Clone,
            Debug,
            PartialEq,
            ::serde::Serialize,
            ::serde::Deserialize,
            ::derive_more::Deref,
            ::derive_more::Display,
            ::derive_more::From,
            ::derive_more::Into,
            ::derive_more::AsRef,
            Hash,
            Eq,
            Ord,
            PartialOrd,
        )]
        pub struct $name(String);

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl core::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl ::redb::Key for $name {
            fn compare(data1: &[u8], data2: &[u8]) -> ::std::cmp::Ordering {
                data1.cmp(data2)
            }
        }

        impl ::redb::Value for $name {
            type SelfType<'a> = $name;

            type AsBytes<'a> = &'a [u8];

            fn fixed_width() -> Option<usize> {
                None
            }

            fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
            where
                Self: 'a,
            {
                Self(String::from_utf8_lossy(data).into())
            }

            fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a> {
                value.0.as_bytes()
            }

            fn type_name() -> ::redb::TypeName {
                ::redb::TypeName::new($redb_type_name)
            }
        }
    };

    ($name:ident, Vec<$type:ty>) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            ::derive_more::Deref,
            ::derive_more::From,
            ::derive_more::Into,
            ::derive_more::AsRef,
            ::derive_more::IntoIterator,
            Hash,
            Eq,
            Ord,
            PartialOrd,
        )]
        pub struct $name(Vec<$type>);
    };
}

#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! impl_manager_message_send {
    ($manager:ident) => {
        impl crate::manager::deferred_sender::ManagerMessageSend<Message>
            for std::sync::Arc<$manager>
        {
            fn send(&self, msgs: SingleOrMany) {
                // just forward to the UDL-generated `send`
                self.send(msgs);
            }
        }
    };
}
