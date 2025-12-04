use crate::{crypto_account::OutputDescriptor, crypto_hdkey::CryptoHdkey, error::UrError};
use pubport::descriptor::ScriptType;

/// crypto-output descriptor (BCR-2020-010)
/// CBOR tag 308
#[derive(Debug, Clone)]
pub struct CryptoOutput {
    pub descriptor: OutputDescriptor,
}

impl CryptoOutput {
    /// Decode from CBOR bytes
    pub fn decode(cbor_bytes: Vec<u8>) -> Result<Self, UrError> {
        let descriptor = crate::crypto_account::decode_output_descriptor(&cbor_bytes)?
            .ok_or_else(|| UrError::CborDecodeError("Unsupported script type".into()))?;
        Ok(Self { descriptor })
    }

    /// Get the script type
    pub fn script_type(&self) -> &ScriptType {
        &self.descriptor.script_type
    }

    /// Get the HD key
    pub fn hdkey(&self) -> &CryptoHdkey {
        &self.descriptor.hdkey
    }

    /// Generate a pubport-compatible descriptor string for this output
    /// Format: script_type([fingerprint/path]xpub/<0;1>/*)
    pub fn descriptor_string(&self, network: bitcoin::Network) -> Result<String, UrError> {
        let hdkey = &self.descriptor.hdkey;
        let xpub = hdkey.to_xpub_string(network)?;

        // get fingerprint from origin or parent_fingerprint
        let fingerprint = hdkey
            .origin
            .as_ref()
            .and_then(|o| o.source_fingerprint)
            .or(hdkey.parent_fingerprint)
            .map(hex::encode)
            .unwrap_or_else(|| "00000000".to_string());

        // get derivation path from origin, or use default for script type
        let deriv_path = hdkey.origin.as_ref().map(|o| o.to_path_string()).unwrap_or_else(|| {
            self.descriptor.script_type.descriptor_derivation_path().to_string()
        });

        // build key expression with origin info and multipath suffix
        let key_expr = format!("[{fingerprint}/{deriv_path}]{xpub}/<0;1>/*");

        let descriptor = self.descriptor.script_type.wrap_with(&key_expr);

        Ok(descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2WPKH crypto-output from Sparrow wallet
    /// Structure: tag(308) tag(404) tag(303) {hdkey_map}
    const P2WPKH_CRYPTO_OUTPUT_HEX: &str = "d90134d90194d9012fa403582102c7e4823730f6ee2cf864e2c352060a88e60b51a84e89e4c8c75ec22590ad6b690458209d2f86043276f9251a4a4f577166a5abeb16b6ec61e226b5b8fa11038bfda42d06d90130a201861831f500f500f5021a37b5eed4081aa80f7cdb";

    #[test]
    fn test_crypto_output_decode_p2wpkh() {
        let cbor = hex::decode(P2WPKH_CRYPTO_OUTPUT_HEX).unwrap();
        let output = CryptoOutput::decode(cbor).unwrap();

        assert_eq!(*output.script_type(), ScriptType::P2wpkh);
        assert_eq!(output.hdkey().key_data.len(), 33);
        assert!(!output.hdkey().is_private);
    }

    /// End-to-end test: CryptoOutput descriptor_string() must be parseable by pubport
    /// This tests the full flow: UR CBOR → CryptoOutput → descriptor string → pubport
    #[test]
    fn test_crypto_output_descriptor_string_parseable_by_pubport() {
        let cbor = hex::decode(P2WPKH_CRYPTO_OUTPUT_HEX).unwrap();
        let output = CryptoOutput::decode(cbor).unwrap();

        let descriptor = output.descriptor_string(bitcoin::Network::Bitcoin).unwrap();
        println!("Generated descriptor: {}", descriptor);

        // pubport expects format like: wpkh([fingerprint/84h/0h/0h]xpub.../<0;1>/*)
        let format = pubport::Format::try_new_from_str(&descriptor);
        assert!(format.is_ok(), "pubport should parse descriptor: {}", descriptor);
    }

    /// Test Sparrow UR with origin keypath (BCR-2020-007 [index, hardened] format)
    /// This UR contains master fingerprint in origin.source_fingerprint
    #[test]
    fn test_sparrow_crypto_output_with_origin() {
        use crate::Ur;

        let ur_string = "UR:CRYPTO-OUTPUT/TAADMWTAADDLOSAOWKAXHDCLAXNSRSIMBNDRBNFTDEJSAXADLSMTWNDSAOWPLBIHFLSBEMLGMWCTDWDSFTFLDACPREAAHDCXMOCXBYKEGWNBDYADGHEMPYCFHGEYRYCATDTIWTWTLBGTSGPEGYECBDDARFHTFNLFAHTAADEHOEADAEAOAEAMTAADDYOTADLNCSGHYKAEYKAEYKAOCYGHENTSDKAXAXAYCYBGKBNBVAASIHFWGAGDEOESCLCFPSPY";

        let ur = Ur::parse(ur_string).expect("UR parse failed");
        let message = ur.message_bytes().expect("No message bytes");
        let output = CryptoOutput::decode(message).expect("CryptoOutput decode failed");
        let hdkey = output.hdkey();

        // must have origin with master fingerprint
        assert!(hdkey.origin.is_some(), "Should have origin keypath");
        let origin = hdkey.origin.as_ref().unwrap();
        assert_eq!(
            origin.source_fingerprint,
            Some([0x54, 0x36, 0xd7, 0x24]),
            "Master fingerprint should be 5436d724"
        );
        assert_eq!(origin.to_path_string(), "84h/0h/0h");

        // descriptor should use master fingerprint and correct xpub
        let descriptor = output.descriptor_string(bitcoin::Network::Bitcoin).unwrap();
        assert!(
            descriptor.contains("5436d724"),
            "Descriptor should have master fingerprint 5436d724, got: {}",
            descriptor
        );
        assert!(
            descriptor.contains("xpub6Bner3L3tdQW367NmmMsWKtMfP7hbu4JxdtbSGdWWjSzLkSUEnT7G9h5GFWUXtifeRhHiUXJuek1qeaTJqnXkveWpiHp8rmt53E8HTMshg9"),
            "Descriptor should have correct xpub, got: {}",
            descriptor
        );

        // should be parseable by pubport
        let format = pubport::Format::try_new_from_str(&descriptor);
        assert!(format.is_ok(), "pubport should parse descriptor: {}", descriptor);
    }
}
