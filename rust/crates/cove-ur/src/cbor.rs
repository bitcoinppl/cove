//! Custom CBOR encode/decode helpers for minicbor derive macros.
//!
//! When using `#[derive(Encode, Decode)]` with minicbor, some fields need
//! custom serialization logic. The `#[cbor(with = "module")]` attribute
//! tells minicbor to use `module::encode` and `module::decode` functions
//! instead of the default behavior.
//!
//! ## Usage
//!
//! ```ignore
//! #[derive(Encode, Decode)]
//! struct MyStruct {
//!     #[cbor(with = "crate::cbor::fingerprint")]
//!     fingerprint: Option<[u8; 4]>,
//! }
//! ```
//!
//! ## Modules
//!
//! - [`fingerprint`]: Encode `[u8; 4]` as big-endian u32 (per BCR spec)
//! - [`keypath_components`]: Decode both BCR path formats, encode as integers
//! - [`embedded_cbor`]: Write pre-encoded CBOR bytes directly (no re-wrapping)
//! - [`optional_bytes`]: Encode `Vec<u8>` as CBOR byte string, not integer array

pub mod embedded_cbor;
pub mod fingerprint;
pub mod keypath_components;
pub mod optional_bytes;
