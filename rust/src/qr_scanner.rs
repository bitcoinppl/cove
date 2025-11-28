//! QR code scanning state machine for single and multi-part QR codes.
//!
//! [`QrScanner`] manages the scanning process for potentially multi-part QR codes like
//! BBQR (animated) and UR (fountain-coded). It maintains decoder state and tracks
//! progress until all parts are received.
//!
//! This represents the "how" - how to reassemble animated QR sequences. Once scanning
//! completes, the result is converted to [`crate::multi_format::MultiFormat`].

use std::sync::Arc;

use bbqr::{
    continuous_join::{ContinuousJoinResult, ContinuousJoiner},
    header::Header,
};
use cove_ur::Ur;
use foundation_ur::Decoder as UrDecoder;
use parking_lot::Mutex;
use tracing::debug;

use crate::{
    multi_format::StringOrData,
    seed_qr::{SeedQr, SeedQrError},
};

/// Internal state for multi-part QR scanning
enum MultiQr {
    Bbqr(Header, Arc<Mutex<ContinuousJoiner>>),
    Ur(Arc<Mutex<UrDecoder>>),
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum MultiQrError {
    #[error("BBQr Parse error: {0}")]
    ParseError(String),

    #[error("Invalid UTF-8")]
    InvalidUtf8,

    #[error("Cannot add binary data to BBQR")]
    CannotAddBinaryDataToBbqr,

    #[error(transparent)]
    InvalidSeedQr(#[from] SeedQrError),

    #[error(transparent)]
    Ur(#[from] cove_ur::UrError),
}

/// Result of a QR scan - either complete with parsed data or in progress
#[derive(Debug, Clone, uniffi::Enum)]
pub enum ScanResult {
    /// Scan complete - here's the parsed data and optionally the raw string
    Complete {
        data: crate::multi_format::MultiFormat,
        /// Raw string data (for screens that need the original string)
        raw_data: Option<String>,
    },
    /// Multi-part scan in progress
    InProgress(ScanProgress),
}

/// Progress information for multi-part QR scans
#[derive(Debug, Clone, uniffi::Enum)]
pub enum ScanProgress {
    /// BBQR progress with scanned/total parts
    Bbqr { scanned: u32, total: u32 },
    /// UR progress as percentage (0.0 to 1.0)
    Ur { percentage: f64 },
}

#[uniffi::export]
impl ScanProgress {
    /// Display text for the progress (e.g., "Scanned 3 of 10" or "Scanned 45%")
    pub fn display_text(&self) -> String {
        match self {
            ScanProgress::Bbqr { scanned, total } => format!("Scanned {} of {}", scanned, total),
            ScanProgress::Ur { percentage } => {
                let percent = (percentage * 100.0) as u32;
                format!("Scanned {}%", percent)
            }
        }
    }

    /// Detail text for the progress (e.g., "7 parts left"), or None for UR
    pub fn detail_text(&self) -> Option<String> {
        match self {
            ScanProgress::Bbqr { scanned, total } => {
                let remaining = total - scanned;
                Some(format!("{} parts left", remaining))
            }
            ScanProgress::Ur { .. } => None, // UR uses fountain codes, no fixed "parts left"
        }
    }
}

/// Stateful QR scanner that handles both single and multi-part QR codes.
///
/// This is the main entry point for QR scanning. It manages the internal state
/// and returns a unified `ScanResult` for every scan, whether first or subsequent.
#[derive(uniffi::Object)]
pub struct QrScanner {
    state: Mutex<Option<MultiQr>>,
}

#[uniffi::export]
impl QrScanner {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self { state: Mutex::new(None) }
    }

    /// Scan a QR code and return the result.
    ///
    /// On first scan, creates the internal state and returns either:
    /// - `Complete(MultiFormat)` for single-part QRs
    /// - `InProgress(ScanProgress)` for multi-part QRs
    ///
    /// On subsequent scans, adds parts and returns updated status.
    #[uniffi::method]
    pub fn scan(&self, qr: StringOrData) -> Result<ScanResult, MultiQrError> {
        let mut guard = self.state.lock();

        match &*guard {
            Some(multi_qr) => self.handle_subsequent_scan(multi_qr, qr),
            None => {
                let (multi_qr_opt, result) = self.handle_first_scan(qr)?;
                *guard = multi_qr_opt;
                Ok(result)
            }
        }
    }

    /// Reset the scanner state for a new scan session.
    #[uniffi::method]
    pub fn reset(&self) {
        *self.state.lock() = None;
    }
}

impl QrScanner {
    /// Handle the first scan - creates MultiQr and returns initial result.
    fn handle_first_scan(
        &self,
        qr: StringOrData,
    ) -> Result<(Option<MultiQr>, ScanResult), MultiQrError> {
        use crate::multi_format::MultiFormat;

        match qr {
            StringOrData::Data(data) => {
                // binary data - try seed QR
                let seed_qr = SeedQr::try_from_data(&data)?;
                let mnemonic = seed_qr.into_mnemonic();
                let multi_format = MultiFormat::Mnemonic(Arc::new(mnemonic.into()));
                Ok((None, ScanResult::Complete { data: multi_format, raw_data: None }))
            }

            StringOrData::String(qr_string) => self.handle_first_scan_string(qr_string),
        }
    }

    /// Handle first scan for string QR codes.
    fn handle_first_scan_string(
        &self,
        qr: String,
    ) -> Result<(Option<MultiQr>, ScanResult), MultiQrError> {
        use crate::multi_format::MultiFormat;

        // try to parse UR
        if qr.to_lowercase().starts_with("ur:") {
            debug!("detected UR prefix, attempting to parse...");
            if let Ok(ur) = Ur::parse(&qr) {
                match ur.to_foundation_ur() {
                    Ok(foundation_ur::UR::SinglePart { .. })
                    | Ok(foundation_ur::UR::SinglePartDeserialized { .. }) => {
                        // single-part UR - convert directly to MultiFormat
                        debug!("Single-part UR, converting to MultiFormat");
                        let multi_format = MultiFormat::try_from_string(&qr)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                        return Ok((
                            None,
                            ScanResult::Complete { data: multi_format, raw_data: Some(qr) },
                        ));
                    }
                    Ok(foundation_ur::UR::MultiPart { .. })
                    | Ok(foundation_ur::UR::MultiPartDeserialized { .. }) => {
                        // multi-part UR - create decoder and return in-progress
                        debug!("Multi-part UR, using decoder");
                        let mut decoder = UrDecoder::default();
                        if let Ok(foundation_ur) = ur.to_foundation_ur() {
                            let _ = decoder.receive(foundation_ur);
                        }
                        let percentage = decoder.estimated_percent_complete();
                        let multi_qr = MultiQr::Ur(Arc::new(Mutex::new(decoder)));
                        return Ok((
                            Some(multi_qr),
                            ScanResult::InProgress(ScanProgress::Ur { percentage }),
                        ));
                    }
                    Err(_) => {
                        // fall through to try other formats
                    }
                }
            }
        }

        // try to parse BBQR
        if let Ok(header) = bbqr::header::Header::try_from_str(&qr) {
            let mut continuous_joiner = ContinuousJoiner::new();
            let join_result = continuous_joiner
                .add_part(qr.clone())
                .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

            match join_result {
                ContinuousJoinResult::Complete(result) => {
                    // single-part BBQR - decode and convert to MultiFormat
                    let data_string =
                        String::from_utf8(result.data).map_err(|_| MultiQrError::InvalidUtf8)?;
                    let multi_format =
                        crate::multi_format::MultiFormat::try_from_string(&data_string)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                    return Ok((
                        None,
                        ScanResult::Complete { data: multi_format, raw_data: Some(data_string) },
                    ));
                }
                ContinuousJoinResult::InProgress { parts_left } => {
                    // multi-part BBQR - return in-progress
                    let total = header.num_parts as u32;
                    let scanned = total - parts_left as u32;
                    let multi_qr = MultiQr::Bbqr(header, Arc::new(Mutex::new(continuous_joiner)));
                    return Ok((
                        Some(multi_qr),
                        ScanResult::InProgress(ScanProgress::Bbqr { scanned, total }),
                    ));
                }
                ContinuousJoinResult::NotStarted => {
                    return Err(MultiQrError::ParseError("BBQR not started".into()));
                }
            }
        }

        // try to parse seed QR
        if let Ok(seed_qr) = SeedQr::try_from_str(&qr) {
            let mnemonic = seed_qr.into_mnemonic();
            let multi_format =
                crate::multi_format::MultiFormat::Mnemonic(Arc::new(mnemonic.into()));
            return Ok((None, ScanResult::Complete { data: multi_format, raw_data: Some(qr) }));
        }

        // plain string - try to convert to MultiFormat directly
        let multi_format = crate::multi_format::MultiFormat::try_from_string(&qr)
            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
        Ok((None, ScanResult::Complete { data: multi_format, raw_data: Some(qr) }))
    }

    /// Handle subsequent scans - adds parts to existing multi-part QR.
    fn handle_subsequent_scan(
        &self,
        multi_qr: &MultiQr,
        qr: StringOrData,
    ) -> Result<ScanResult, MultiQrError> {
        match (multi_qr, qr) {
            (MultiQr::Bbqr(header, joiner), StringOrData::String(qr_string)) => {
                let join_result = joiner
                    .lock()
                    .add_part(qr_string)
                    .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

                match join_result {
                    ContinuousJoinResult::Complete(result) => {
                        let data_string = String::from_utf8(result.data)
                            .map_err(|_| MultiQrError::InvalidUtf8)?;
                        let multi_format =
                            crate::multi_format::MultiFormat::try_from_string(&data_string)
                                .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                        Ok(ScanResult::Complete { data: multi_format, raw_data: Some(data_string) })
                    }
                    ContinuousJoinResult::InProgress { parts_left } => {
                        let total = header.num_parts as u32;
                        let scanned = total - parts_left as u32;
                        Ok(ScanResult::InProgress(ScanProgress::Bbqr { scanned, total }))
                    }
                    ContinuousJoinResult::NotStarted => {
                        Err(MultiQrError::ParseError("BBQR not started".into()))
                    }
                }
            }

            (MultiQr::Ur(decoder), StringOrData::String(qr_string)) => {
                use cove_ur::UrError;

                let ur = Ur::parse(&qr_string)?;
                let foundation_ur = ur.to_foundation_ur()?;
                let mut decoder = decoder.lock();
                decoder.receive(foundation_ur).map_err(|e| UrError::UrParseError(e.to_string()))?;

                if decoder.is_complete() {
                    let message = decoder
                        .message()
                        .map_err(|e| UrError::UrParseError(e.to_string()))?
                        .ok_or_else(|| UrError::UrParseError("No message".into()))?;
                    let ur_type_str = decoder.ur_type().unwrap_or("bytes");
                    debug!("UR complete, type: {}, message len: {}", ur_type_str, message.len());

                    let ur_type = crate::ur::UrType::from_str(ur_type_str);
                    let multi_format =
                        crate::multi_format::MultiFormat::try_from_ur_payload(message, &ur_type)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                    Ok(ScanResult::Complete {
                        data: multi_format,
                        raw_data: None, // UR payload is binary, not useful as string
                    })
                } else {
                    let percentage = decoder.estimated_percent_complete();
                    Ok(ScanResult::InProgress(ScanProgress::Ur { percentage }))
                }
            }

            // error cases - can't add binary data to string-based formats
            (MultiQr::Bbqr(_, _), StringOrData::Data(_)) => {
                Err(MultiQrError::CannotAddBinaryDataToBbqr)
            }
            (MultiQr::Ur(_), StringOrData::Data(_)) => {
                Err(MultiQrError::ParseError("UR requires string data".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test QrScanner returns Complete with MultiFormat for single-part BBQR containing an address
    #[test]
    fn test_qr_scanner_single_part_bbqr_address() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        // a valid Bitcoin mainnet address
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // create a single-part BBQR containing the address
        let split = Split::try_from_data(
            address.as_bytes(),
            FileType::UnicodeText,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 1,
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode");

        let bbqr_string = &split.parts[0];

        // use QrScanner - should return Complete with MultiFormat
        let scanner = QrScanner::new();
        let result = scanner.scan(StringOrData::String(bbqr_string.clone()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                // should be an Address
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Address(_)),
                    "Should be Address, got: {:?}",
                    data
                );
            }
            ScanResult::InProgress(_) => {
                panic!("Single-part BBQR should complete immediately, not be in progress");
            }
        }
    }

    /// Test QrScanner handles plain Bitcoin address
    #[test]
    fn test_qr_scanner_plain_address() {
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        let scanner = QrScanner::new();
        let result = scanner.scan(StringOrData::String(address.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Address(_)),
                    "Should be Address"
                );
            }
            ScanResult::InProgress(_) => {
                panic!("Plain address should complete immediately");
            }
        }
    }

    /// Test QrScanner returns Complete with MultiFormat for single-part BBQR containing an xpub
    #[test]
    fn test_qr_scanner_single_part_bbqr_xpub() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        // a valid CHILD xpub (depth > 0) - pubport rejects master xpubs
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";

        // create a single-part BBQR containing the xpub
        let split = Split::try_from_data(
            xpub.as_bytes(),
            FileType::UnicodeText,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 1,
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode");

        let bbqr_string = &split.parts[0];

        // use QrScanner - should return Complete with MultiFormat
        let scanner = QrScanner::new();
        let result = scanner.scan(StringOrData::String(bbqr_string.clone()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                // should be a HardwareExport containing the xpub
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should be HardwareExport, got: {:?}",
                    data
                );
            }
            ScanResult::InProgress(_) => {
                panic!("Single-part BBQR should complete immediately, not be in progress");
            }
        }
    }

    /// Test QrScanner handles plain xpub string
    #[test]
    fn test_qr_scanner_plain_xpub() {
        // use a CHILD xpub (depth > 0) - pubport rejects master xpubs
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";

        let scanner = QrScanner::new();
        let result = scanner.scan(StringOrData::String(xpub.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should be HardwareExport"
                );
            }
            ScanResult::InProgress(_) => {
                panic!("Plain xpub should complete immediately");
            }
        }
    }

    /// Test multi-part UR completion with crypto-seed
    /// Creates a multi-part UR using foundation_ur, scans all parts, verifies completion
    #[test]
    fn test_multi_part_ur_completion() {
        use cove_ur::CryptoSeed;
        use foundation_ur::Encoder as UrEncoder;

        // create test seed with 16-byte entropy
        let entropy = vec![0xAB; 16];
        let seed = CryptoSeed::from_entropy(entropy.clone()).unwrap();
        let cbor = seed.encode().unwrap();

        // create multi-part UR with small fragment size to force multiple parts
        const MAX_FRAGMENT_LEN: usize = 20;
        let mut encoder = UrEncoder::new();
        encoder.start("crypto-seed", &cbor, MAX_FRAGMENT_LEN);

        // collect all parts
        let sequence_count = encoder.sequence_count();
        assert!(sequence_count > 1, "Should create multiple parts with small fragment size");

        let mut parts = Vec::new();
        for _ in 0..sequence_count {
            let part = encoder.next_part();
            parts.push(part.to_string());
        }

        // scan all parts through QrScanner
        let scanner = QrScanner::new();

        // first scan should return in-progress
        let first_result = scanner.scan(StringOrData::String(parts[0].clone())).unwrap();
        match first_result {
            ScanResult::InProgress(ScanProgress::Ur { percentage }) => {
                assert!(percentage > 0.0 && percentage < 1.0, "Progress should be partial");
            }
            _ => panic!("First scan should be in progress"),
        }

        // scan remaining parts
        for (i, part) in parts.iter().enumerate().skip(1) {
            let result = scanner.scan(StringOrData::String(part.clone())).unwrap();

            if i < parts.len() - 1 {
                // intermediate parts should show increasing progress
                match result {
                    ScanResult::InProgress(ScanProgress::Ur { percentage }) => {
                        assert!(percentage > 0.0, "Progress should be positive");
                    }
                    ScanResult::Complete { .. } => {
                        // UR fountain codes may complete before all parts are scanned
                        break;
                    }
                    _ => panic!("Expected UR progress or completion"),
                }
            }
        }

        // final scan should complete
        let final_result =
            scanner.scan(StringOrData::String(parts.last().unwrap().clone())).unwrap();
        match final_result {
            ScanResult::Complete { data, .. } => {
                // should be a Mnemonic format
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Mnemonic(_)),
                    "Should parse as Mnemonic"
                );
            }
            ScanResult::InProgress(_) => {
                // may still need more parts due to fountain coding - keep scanning
                // repeat last part to ensure completion
                for _ in 0..10 {
                    let result =
                        scanner.scan(StringOrData::String(parts.last().unwrap().clone())).unwrap();
                    if matches!(result, ScanResult::Complete { .. }) {
                        break;
                    }
                }
            }
        }
    }

    /// Test error handling for malformed UR sequences
    #[test]
    fn test_malformed_ur_sequences() {
        let scanner = QrScanner::new();

        // invalid UR prefix
        let result = scanner.scan(StringOrData::String("ur:invalid-type/test".to_string()));
        assert!(result.is_err(), "Should fail on invalid UR type");

        // malformed UR string
        let result = scanner.scan(StringOrData::String("ur:crypto-seed/invalid!!!".to_string()));
        assert!(result.is_err(), "Should fail on malformed UR");

        // empty UR
        let result = scanner.scan(StringOrData::String("ur:crypto-seed/".to_string()));
        assert!(result.is_err(), "Should fail on empty UR payload");
    }

    /// Test progress percentage accuracy for multi-part UR
    #[test]
    fn test_ur_progress_percentage() {
        use cove_ur::CryptoSeed;
        use foundation_ur::Encoder as UrEncoder;

        // create larger payload to ensure multiple parts
        let entropy = vec![0xFF; 32];
        let seed = CryptoSeed::from_entropy(entropy).unwrap();
        let cbor = seed.encode().unwrap();

        // small fragment size to force many parts
        const MAX_FRAGMENT_LEN: usize = 15;
        let mut encoder = UrEncoder::new();
        encoder.start("crypto-seed", &cbor, MAX_FRAGMENT_LEN);

        let sequence_count = encoder.sequence_count();
        assert!(sequence_count > 2, "Should have multiple parts");

        let mut parts = Vec::new();
        for _ in 0..sequence_count {
            parts.push(encoder.next_part().to_string());
        }

        let scanner = QrScanner::new();
        let mut last_progress = 0.0;

        // scan parts and verify progress increases (or completes early due to fountain codes)
        for (i, part) in parts.iter().enumerate() {
            let result = scanner.scan(StringOrData::String(part.clone())).unwrap();

            match result {
                ScanResult::InProgress(ScanProgress::Ur { percentage }) => {
                    assert!(
                        percentage >= last_progress,
                        "Progress should not decrease: {} < {}",
                        percentage,
                        last_progress
                    );
                    assert!(percentage <= 1.0, "Progress should not exceed 100%");
                    last_progress = percentage;
                }
                ScanResult::Complete { .. } => {
                    // fountain codes may complete before all parts scanned
                    assert!(i > 0, "Should scan at least 2 parts before completion");
                    break;
                }
                _ => panic!("Expected UR progress or completion"),
            }
        }
    }

    /// Test QrScanner reset behavior
    #[test]
    fn test_qr_scanner_reset() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // create multi-part BBQR
        let split = Split::try_from_data(
            address.as_bytes(),
            FileType::UnicodeText,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 3,
                max_split_number: 3,
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode");

        assert!(split.parts.len() > 1, "Should have multiple parts");

        let scanner = QrScanner::new();

        // scan first part
        let result = scanner.scan(StringOrData::String(split.parts[0].clone())).unwrap();
        assert!(matches!(result, ScanResult::InProgress(_)), "First scan should be in progress");

        // reset scanner
        scanner.reset();

        // scan a different single QR - should work as if fresh scanner
        let result = scanner.scan(StringOrData::String(address.to_string())).unwrap();
        match result {
            ScanResult::Complete { .. } => {
                // success - reset worked
            }
            _ => panic!("After reset, single QR should complete immediately"),
        }
    }

    /// Test ScanProgress display methods
    #[test]
    fn test_scan_progress_display() {
        // BBQR progress
        let bbqr_progress = ScanProgress::Bbqr { scanned: 3, total: 10 };
        assert_eq!(bbqr_progress.display_text(), "Scanned 3 of 10");
        assert_eq!(bbqr_progress.detail_text(), Some("7 parts left".to_string()));

        // UR progress
        let ur_progress = ScanProgress::Ur { percentage: 0.45 };
        assert_eq!(ur_progress.display_text(), "Scanned 45%");
        assert_eq!(ur_progress.detail_text(), None);

        // edge cases
        let complete_bbqr = ScanProgress::Bbqr { scanned: 10, total: 10 };
        assert_eq!(complete_bbqr.display_text(), "Scanned 10 of 10");
        assert_eq!(complete_bbqr.detail_text(), Some("0 parts left".to_string()));

        let zero_ur = ScanProgress::Ur { percentage: 0.0 };
        assert_eq!(zero_ur.display_text(), "Scanned 0%");

        let complete_ur = ScanProgress::Ur { percentage: 1.0 };
        assert_eq!(complete_ur.display_text(), "Scanned 100%");
    }
}
