use bitcoin::secp256k1::hashes::sha256::Hash as Sha256Hash;
use std::hash::{DefaultHasher, Hasher as _};
 
uniffi::setup_scaffolding!();

 pub fn calculate_hash<T>(t: &T) -> u64
 where
     T: std::hash::Hash + ?Sized,
 {
     let mut s = DefaultHasher::new();
     t.hash(&mut s);
     s.finish()
 }

 pub fn generate_random_chain_code() -> [u8; 32] {
     use rand::Rng as _;

     let rng = &mut rand::rng();
     let mut chain_code = [0u8; 32];
     rng.fill(&mut chain_code);

     chain_code
 }

 pub fn sha256_hash(bytes: &[u8]) -> Sha256Hash {
     use bitcoin::hashes::Hash as _;
     Sha256Hash::hash(bytes)
 }

 pub fn message_digest(message: &[u8]) -> bitcoin::secp256k1::Message {
     let hash = sha256_hash(message);
     let digest: &[u8; 32] = hash.as_ref();
     bitcoin::secp256k1::Message::from_digest_slice(digest).expect("just hashed so hash is 32 bytes")
 }

 mod ffi {
     #[uniffi::export]
     fn hex_encode(bytes: Vec<u8>) -> String {
         hex::encode(bytes)
     }

     #[uniffi::export]
     fn hex_decode(hex: &str) -> Option<Vec<u8>> {
         hex::decode(hex).ok()
     }

     #[uniffi::export]
     pub fn generate_random_chain_code() -> String {
         use rand::Rng as _;

         let rng = &mut rand::rng();
         let mut chain_code = [0u8; 32];
         rng.fill(&mut chain_code);

         hex::encode(chain_code)
     }

     #[uniffi::export]
     fn hex_to_utf8_string(hex: &str) -> Option<String> {
         let bytes = hex_decode(hex)?;
         String::from_utf8(bytes).ok()
     }
 }