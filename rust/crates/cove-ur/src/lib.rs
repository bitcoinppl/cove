pub mod coin_info;
pub mod crypto_account;
pub mod crypto_hdkey;
pub mod crypto_output;
pub mod crypto_psbt;
pub mod crypto_seed;
pub mod error;
pub mod keypath;
pub mod registry;
pub mod ur;

pub use coin_info::CryptoCoinInfo;
pub use crypto_account::{CryptoAccount, OutputDescriptor};
pub use crypto_hdkey::CryptoHdkey;
pub use crypto_output::CryptoOutput;
pub use crypto_psbt::CryptoPsbt;
pub use crypto_seed::CryptoSeed;
pub use error::{Result, UrError};
pub use keypath::CryptoKeypath;
pub use ur::Ur;

// UniFFI scaffolding
uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use foundation_ur::{UR, bytewords};

    /// Test ur:bytes with JSON payload (Passport wallet export format)
    ///
    /// This tests the encoding/decoding of JSON wallet data wrapped in ur:bytes,
    /// which is the format used by Foundation Passport for generic wallet export.
    ///
    /// JSON format matches: https://github.com/Foundation-Devices/passport2
    #[test]
    fn test_ur_bytes_json_passport_format() {
        // Passport-style JSON wallet export using the "abandon" seed
        // Master fingerprint: 73c5da0a
        // BIP84 zpub from "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        let passport_json = r#"{
  "xfp": "73c5da0a",
  "bip84": {
    "deriv": "m/84'/0'/0'",
    "xpub": "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1ADqtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs",
    "first": "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
  }
}"#;

        // encode JSON as ur:bytes
        // ur:bytes is simple CBOR byte string (no tag) wrapped in UR format
        let json_bytes = passport_json.as_bytes();

        // create CBOR-encoded bytes (just the raw byte string, no tag for "bytes" type)
        let mut cbor = Vec::new();
        let mut encoder = minicbor::Encoder::new(&mut cbor);
        encoder.bytes(json_bytes).unwrap();

        // create UR
        let ur = UR::new("bytes", &cbor);
        let ur_string = ur.to_string();

        // verify it starts with ur:bytes/
        assert!(ur_string.starts_with("ur:bytes/"), "UR string should start with ur:bytes/");

        // decode UR string
        let parsed_ur = UR::parse(&ur_string).unwrap();
        assert_eq!(parsed_ur.as_type(), "bytes");

        // extract CBOR bytes from UR
        let cbor_bytes = match parsed_ur {
            UR::SinglePart { message, .. } => {
                bytewords::decode(message, bytewords::Style::Minimal).unwrap()
            }
            UR::SinglePartDeserialized { message, .. } => message.to_vec(),
            _ => panic!("Expected single-part UR"),
        };

        // decode CBOR to get JSON bytes
        let mut decoder = minicbor::Decoder::new(&cbor_bytes);
        let decoded_bytes = decoder.bytes().unwrap();

        // verify we get the original JSON back
        let decoded_json = std::str::from_utf8(decoded_bytes).unwrap();
        assert_eq!(decoded_json, passport_json);

        // parse the JSON and verify key fields
        let parsed: serde_json::Value = serde_json::from_str(decoded_json).unwrap();
        assert_eq!(parsed["xfp"].as_str().unwrap(), "73c5da0a");
        assert_eq!(parsed["bip84"]["deriv"].as_str().unwrap(), "m/84'/0'/0'");
        assert!(parsed["bip84"]["xpub"].as_str().unwrap().starts_with("zpub"));
        assert!(parsed["bip84"]["first"].as_str().unwrap().starts_with("bc1q"));
    }
}
