pub mod header;
pub mod ndef_type;
pub mod parser;
pub mod payload;
pub mod record;

pub struct NfcReader {}

pub struct NdefMessage {}

pub struct NdefRecord {}

uniffi::setup_scaffolding!();
