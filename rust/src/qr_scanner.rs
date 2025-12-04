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

// ============================================================================
// State machine types
// ============================================================================

/// QR scanner state machine (pure Rust, not exported via FFI)
enum QrScanner {
    Uninitialized,
    InProgress(MultiQr),
    Complete(ScanResult),
}

/// In-progress multi-part QR state
enum MultiQr {
    Bbqr(BbqrInProgress),
    Ur(Box<UrInProgress>),
}

/// BBQr scanning in progress
struct BbqrInProgress {
    header: Header,
    joiner: ContinuousJoiner,
    previous_progress: Option<ScanProgress>,
}

/// UR scanning in progress
struct UrInProgress {
    decoder: UrDecoder,
    previous_progress: Option<ScanProgress>,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum MultiQrError {
    #[error("{0}")]
    ParseError(String),

    #[error("Invalid UTF-8")]
    InvalidUtf8,

    #[error("Multi-part QR requires string data, not binary")]
    RequiresStringData,

    #[error(transparent)]
    InvalidSeedQr(#[from] SeedQrError),

    #[error(transparent)]
    Ur(#[from] cove_ur::UrError),

    #[error("BBQr CBOR file type is not yet supported")]
    BbqrCborNotSupported,
}

impl StringOrData {
    /// Convert to string, returning an error if this is binary data.
    fn into_string(self) -> Result<String, MultiQrError> {
        match self {
            Self::String(s) => Ok(s),
            Self::Data(_) => Err(MultiQrError::RequiresStringData),
        }
    }
}

/// Haptic feedback hint for the platform to trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HapticFeedback {
    /// Light tap - new part scanned in multi-part QR
    Progress,
    /// Success notification - scan complete (single or multi-part)
    Success,
    /// No haptic feedback (duplicate part, no change)
    None,
}

/// Result of a QR scan - either complete with parsed data or in progress
#[derive(Debug, Clone, uniffi::Enum)]
pub enum ScanResult {
    /// Scan complete - here's the parsed data
    Complete {
        data: crate::multi_format::MultiFormat,
        /// Haptic feedback to trigger
        haptic: HapticFeedback,
    },
    /// Multi-part scan in progress
    InProgress {
        progress: ScanProgress,
        /// Haptic feedback to trigger
        haptic: HapticFeedback,
    },
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
                if remaining == 1 {
                    Some("1 part left".to_string())
                } else {
                    Some(format!("{} parts left", remaining))
                }
            }
            ScanProgress::Ur { .. } => None, // UR uses fountain codes, no fixed "parts left"
        }
    }
}

impl ScanProgress {
    /// Check if this progress is greater than another (used for haptic feedback)
    fn is_greater_than(&self, other: &ScanProgress) -> bool {
        match (self, other) {
            (ScanProgress::Bbqr { scanned: a, .. }, ScanProgress::Bbqr { scanned: b, .. }) => a > b,
            (ScanProgress::Ur { percentage: a }, ScanProgress::Ur { percentage: b }) => a > b,
            // different types shouldn't happen, but treat as progress
            _ => true,
        }
    }
}

// ============================================================================
// FFI wrapper (exposed to Swift/Kotlin as "QrScanner")
// ============================================================================

/// FFI wrapper for QrScanner state machine.
///
/// This is the main entry point for QR scanning from Swift/Kotlin.
/// It wraps the internal state machine in a Mutex for thread safety.
#[derive(uniffi::Object)]
#[uniffi(name = "QrScanner")]
pub struct QrScannerFFI(Arc<Mutex<QrScanner>>);

#[uniffi::export(name = "QrScanner")]
impl QrScannerFFI {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(QrScanner::Uninitialized)))
    }

    /// Scan a QR code and return the result.
    ///
    /// On first scan, detects the format and returns either:
    /// - `Complete(MultiFormat)` for single-part QRs
    /// - `InProgress(ScanProgress)` for multi-part QRs (BBQr or UR)
    ///
    /// On subsequent scans, adds parts and returns updated status.
    /// The haptic field indicates what feedback the platform should trigger.
    #[uniffi::method]
    pub fn scan(&self, qr: StringOrData) -> Result<ScanResult, MultiQrError> {
        self.0.lock().scan(qr)
    }

    /// Reset the scanner state for a new scan session.
    #[uniffi::method]
    pub fn reset(&self) {
        self.0.lock().reset()
    }
}

/// Parse a UR QR code (caller already verified `ur:` prefix).
/// Returns the new state and the scan result.
fn parse_ur(qr: &str) -> Result<(QrScanner, ScanResult), MultiQrError> {
    use crate::multi_format::MultiFormat;

    debug!("detected UR prefix, attempting to parse...");
    let ur = Ur::parse(qr)?;

    match ur.to_foundation_ur()? {
        foundation_ur::UR::SinglePart { .. } | foundation_ur::UR::SinglePartDeserialized { .. } => {
            debug!("Single-part UR, converting to MultiFormat");
            let multi_format = MultiFormat::try_from_string(qr)
                .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

            let result =
                ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
            Ok((QrScanner::Complete(result.clone()), result))
        }

        foundation_ur::UR::MultiPart { .. } | foundation_ur::UR::MultiPartDeserialized { .. } => {
            use cove_ur::UrError;

            debug!("Multi-part UR, using decoder");
            let mut decoder = UrDecoder::default();
            let foundation_ur = ur.to_foundation_ur()?;
            decoder.receive(foundation_ur).map_err(|e| UrError::UrParseError(e.to_string()))?;

            let percentage = decoder.estimated_percent_complete();
            let progress = ScanProgress::Ur { percentage };
            let result = ScanResult::InProgress { progress, haptic: HapticFeedback::Progress };

            let state = QrScanner::InProgress(MultiQr::Ur(Box::new(UrInProgress {
                decoder,
                previous_progress: Some(ScanProgress::Ur { percentage }),
            })));

            Ok((state, result))
        }
    }
}

/// Parse completed BBQr data based on file type.
///
/// Binary types (Transaction, Psbt) are parsed directly from bytes.
/// Text types (UnicodeText, Json) are converted to UTF-8 first.
/// CBOR is not yet supported and returns an error.
fn parse_bbqr_data(
    data: Vec<u8>,
    file_type: bbqr::file_type::FileType,
) -> Result<crate::multi_format::MultiFormat, MultiQrError> {
    use crate::multi_format::MultiFormat;
    use crate::transaction::ffi::BitcoinTransaction;
    use bbqr::file_type::FileType;

    match file_type {
        FileType::Transaction => {
            MultiFormat::try_from_data(&data).map_err(|e| MultiQrError::ParseError(e.to_string()))
        }

        FileType::Psbt => {
            // parse raw PSBT bytes and extract the unsigned transaction
            let crypto_psbt = cove_ur::CryptoPsbt::from_psbt_bytes(data)
                .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

            let psbt = crypto_psbt.psbt();
            let unsigned_tx = &psbt.unsigned_tx;
            let tx_bytes = bitcoin::consensus::serialize(unsigned_tx);

            let txn = BitcoinTransaction::try_from_data(&tx_bytes)
                .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

            Ok(MultiFormat::Transaction(Arc::new(txn)))
        }

        FileType::Cbor => Err(MultiQrError::BbqrCborNotSupported),

        FileType::UnicodeText | FileType::Json => {
            let data_string = String::from_utf8(data).map_err(|_| MultiQrError::InvalidUtf8)?;
            MultiFormat::try_from_string(&data_string)
                .map_err(|e| MultiQrError::ParseError(e.to_string()))
        }
    }
}

/// Parse a BBQr QR code (caller already verified header parses).
/// Returns the new state and the scan result.
fn parse_bbqr(qr: &str, header: Header) -> Result<(QrScanner, ScanResult), MultiQrError> {
    let mut joiner = ContinuousJoiner::new();
    let join_result =
        joiner.add_part(qr.to_string()).map_err(|e| MultiQrError::ParseError(e.to_string()))?;

    match join_result {
        ContinuousJoinResult::Complete(result) => {
            let multi_format = parse_bbqr_data(result.data, header.file_type)?;
            let result =
                ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
            Ok((QrScanner::Complete(result.clone()), result))
        }

        ContinuousJoinResult::InProgress { parts_left } => {
            let total = header.num_parts as u32;
            let scanned = total - parts_left as u32;
            let progress = ScanProgress::Bbqr { scanned, total };
            let result = ScanResult::InProgress {
                progress: progress.clone(),
                haptic: HapticFeedback::Progress,
            };

            let state = QrScanner::InProgress(MultiQr::Bbqr(BbqrInProgress {
                header,
                joiner,
                previous_progress: Some(progress),
            }));

            Ok((state, result))
        }

        ContinuousJoinResult::NotStarted => {
            Err(MultiQrError::ParseError("BBQR not started".into()))
        }
    }
}

impl QrScanner {
    /// Scan a QR code and return the result.
    ///
    /// State transitions:
    /// - Uninitialized → Complete (single-part) or InProgress (multi-part)
    /// - InProgress(Bbqr) → InProgress (more parts) or Complete
    /// - InProgress(Ur) → InProgress (more parts) or Complete
    /// - Complete → returns cached result (call `reset()` to scan again)
    fn scan(&mut self, qr: StringOrData) -> Result<ScanResult, MultiQrError> {
        // if already complete, return cached result without modifying state
        if let Self::Complete(result) = self {
            return Ok(result.clone());
        }

        // take ownership of current state to avoid borrow issues
        let current_state = std::mem::replace(self, Self::Uninitialized);

        let (new_state, result) = match current_state {
            Self::Uninitialized => Self::scan_first(qr)?,

            Self::InProgress(MultiQr::Bbqr(bbqr)) => {
                let qr = qr.into_string()?;
                Self::scan_bbqr_part(bbqr, &qr)?
            }

            Self::InProgress(MultiQr::Ur(ur)) => {
                let qr = qr.into_string()?;
                Self::scan_ur_part(*ur, &qr)?
            }

            Self::Complete(_) => unreachable!("handled above"),
        };

        *self = new_state;
        Ok(result)
    }

    /// Reset the scanner to uninitialized state.
    fn reset(&mut self) {
        *self = Self::Uninitialized;
    }

    /// Handle the first scan - detects format and returns new state.
    fn scan_first(qr: StringOrData) -> Result<(Self, ScanResult), MultiQrError> {
        use crate::multi_format::MultiFormat;

        // binary data - can only be SeedQR
        let qr = match qr {
            StringOrData::Data(data) => {
                let seed_qr = SeedQr::try_from_data(&data)?;
                let mnemonic = seed_qr.into_mnemonic();
                let multi_format = MultiFormat::Mnemonic(Arc::new(mnemonic.into()));
                let result =
                    ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
                return Ok((Self::Complete(result.clone()), result));
            }
            StringOrData::String(s) => s,
        };

        // UR format
        if qr.to_lowercase().starts_with("ur:") {
            return parse_ur(&qr);
        }

        // BBQr format
        if let Ok(header) = Header::try_from_str(&qr) {
            return parse_bbqr(&qr, header);
        }

        // SeedQR (numeric string)
        if let Ok(seed_qr) = SeedQr::try_from_str(&qr) {
            let mnemonic = seed_qr.into_mnemonic();
            let multi_format = MultiFormat::Mnemonic(Arc::new(mnemonic.into()));
            let result =
                ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
            return Ok((Self::Complete(result.clone()), result));
        }

        // plain string - address, xpub, etc.
        let multi_format = MultiFormat::try_from_string(&qr)
            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
        let result = ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
        Ok((Self::Complete(result.clone()), result))
    }

    /// Process a BBQr part and return new state.
    fn scan_bbqr_part(
        mut bbqr: BbqrInProgress,
        qr: &str,
    ) -> Result<(Self, ScanResult), MultiQrError> {
        let join_result = bbqr
            .joiner
            .add_part(qr.to_string())
            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

        match join_result {
            ContinuousJoinResult::Complete(result) => {
                let multi_format = parse_bbqr_data(result.data, bbqr.header.file_type)?;
                let result =
                    ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
                Ok((Self::Complete(result.clone()), result))
            }
            ContinuousJoinResult::InProgress { parts_left } => {
                let total = bbqr.header.num_parts as u32;
                let scanned = total - parts_left as u32;
                let progress = ScanProgress::Bbqr { scanned, total };

                // determine haptic feedback based on progress change
                let haptic = match &bbqr.previous_progress {
                    Some(prev) if progress.is_greater_than(prev) => HapticFeedback::Progress,
                    Some(_) => HapticFeedback::None, // duplicate part
                    None => HapticFeedback::Progress,
                };

                bbqr.previous_progress = Some(progress.clone());
                let result = ScanResult::InProgress { progress, haptic };
                Ok((Self::InProgress(MultiQr::Bbqr(bbqr)), result))
            }
            ContinuousJoinResult::NotStarted => {
                Err(MultiQrError::ParseError("BBQR not started".into()))
            }
        }
    }

    /// Process a UR part and return new state.
    fn scan_ur_part(mut ur: UrInProgress, qr: &str) -> Result<(Self, ScanResult), MultiQrError> {
        use cove_ur::UrError;

        let parsed_ur = Ur::parse(qr)?;
        let foundation_ur = parsed_ur.to_foundation_ur()?;
        ur.decoder.receive(foundation_ur).map_err(|e| UrError::UrParseError(e.to_string()))?;

        if ur.decoder.is_complete() {
            let message = ur
                .decoder
                .message()
                .map_err(|e| UrError::UrParseError(e.to_string()))?
                .ok_or_else(|| UrError::UrParseError("No message".into()))?;
            let ur_type_str = ur.decoder.ur_type().unwrap_or("bytes");
            debug!("UR complete, type: {}, message len: {}", ur_type_str, message.len());

            let ur_type = crate::ur::UrType::from_str(ur_type_str);
            let multi_format =
                crate::multi_format::MultiFormat::try_from_ur_payload(message, &ur_type)
                    .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
            let result =
                ScanResult::Complete { data: multi_format, haptic: HapticFeedback::Success };
            Ok((Self::Complete(result.clone()), result))
        } else {
            let percentage = ur.decoder.estimated_percent_complete();
            let progress = ScanProgress::Ur { percentage };

            // determine haptic feedback based on progress change
            let haptic = match &ur.previous_progress {
                Some(prev) if progress.is_greater_than(prev) => HapticFeedback::Progress,
                Some(_) => HapticFeedback::None, // duplicate part
                None => HapticFeedback::Progress,
            };

            ur.previous_progress = Some(progress.clone());
            let result = ScanResult::InProgress { progress, haptic };
            Ok((Self::InProgress(MultiQr::Ur(Box::new(ur))), result))
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
        let scanner = QrScannerFFI::new();
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
            ScanResult::InProgress { .. } => {
                panic!("Single-part BBQR should complete immediately, not be in progress");
            }
        }
    }

    /// Test QrScanner handles plain Bitcoin address
    #[test]
    fn test_qr_scanner_plain_address() {
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(address.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Address(_)),
                    "Should be Address"
                );
            }
            ScanResult::InProgress { .. } => {
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
        let scanner = QrScannerFFI::new();
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
            ScanResult::InProgress { .. } => {
                panic!("Single-part BBQR should complete immediately, not be in progress");
            }
        }
    }

    /// Test QrScanner handles plain xpub string
    #[test]
    fn test_qr_scanner_plain_xpub() {
        // use a CHILD xpub (depth > 0) - pubport rejects master xpubs
        let xpub = "xpub6CiKnWv7PPyyeb4kCwK4fidKqVjPfD9TP6MiXnzBVGZYNanNdY3mMvywcrdDc6wK82jyBSd95vsk26QujnJWPrSaPfYeyW7NyX37HHGtfQM";

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(xpub.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should be HardwareExport"
                );
            }
            ScanResult::InProgress { .. } => {
                panic!("Plain xpub should complete immediately");
            }
        }
    }

    /// Test multi-part UR: progress tracking, completion, and result parsing
    #[test]
    fn test_multi_part_ur() {
        use cove_ur::CryptoSeed;
        use foundation_ur::Encoder as UrEncoder;

        // larger entropy + small fragments = more parts for thorough testing
        let entropy = vec![0xFF; 32];
        let seed = CryptoSeed::from_entropy(entropy).unwrap();
        let cbor = seed.encode().unwrap();

        const MAX_FRAGMENT_LEN: usize = 15;
        let mut encoder = UrEncoder::new();
        encoder.start("crypto-seed", &cbor, MAX_FRAGMENT_LEN);

        let sequence_count = encoder.sequence_count();
        assert!(sequence_count > 2, "Should have multiple parts");

        let mut parts = Vec::new();
        for _ in 0..sequence_count {
            parts.push(encoder.next_part().to_string());
        }

        let scanner = QrScannerFFI::new();
        let mut last_progress = 0.0;
        let mut completed = false;

        for (i, part) in parts.iter().enumerate() {
            let result = scanner.scan(StringOrData::String(part.clone())).unwrap();

            match result {
                ScanResult::InProgress { progress: ScanProgress::Ur { percentage }, .. } => {
                    assert!(
                        percentage >= last_progress,
                        "Progress should not decrease: {} < {}",
                        percentage,
                        last_progress
                    );
                    assert!(percentage > 0.0 && percentage <= 1.0, "Progress should be in (0, 1]");
                    last_progress = percentage;
                }
                ScanResult::Complete { data, .. } => {
                    assert!(i > 0, "Should scan at least 2 parts before completion");
                    assert!(
                        matches!(data, crate::multi_format::MultiFormat::Mnemonic(_)),
                        "Should parse as Mnemonic"
                    );
                    completed = true;
                    break;
                }
                _ => panic!("Expected UR progress or completion"),
            }
        }

        assert!(completed, "UR should complete after scanning all parts");
    }

    /// Test error handling for malformed UR sequences
    #[test]
    fn test_malformed_ur_sequences() {
        let scanner = QrScannerFFI::new();

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

        let scanner = QrScannerFFI::new();

        // scan first part
        let result = scanner.scan(StringOrData::String(split.parts[0].clone())).unwrap();
        assert!(
            matches!(result, ScanResult::InProgress { .. }),
            "First scan should be in progress"
        );

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
        let one_left = ScanProgress::Bbqr { scanned: 9, total: 10 };
        assert_eq!(one_left.detail_text(), Some("1 part left".to_string()));

        let complete_bbqr = ScanProgress::Bbqr { scanned: 10, total: 10 };
        assert_eq!(complete_bbqr.display_text(), "Scanned 10 of 10");
        assert_eq!(complete_bbqr.detail_text(), Some("0 parts left".to_string()));

        let zero_ur = ScanProgress::Ur { percentage: 0.0 };
        assert_eq!(zero_ur.display_text(), "Scanned 0%");

        let complete_ur = ScanProgress::Ur { percentage: 1.0 };
        assert_eq!(complete_ur.display_text(), "Scanned 100%");
    }

    /// Test PSBT hex constant used in tests
    const TEST_PSBT_HEX: &str = "70736274ff01009a020000000258e87a21b56daf0c23be8e7070456c336f7cbaa5c8757924f545887bb2abdd750000000000ffffffff838d0427d0ec650a68aa46bb0b098aea4422c071b2ca78352a077959d07cea1d0100000000ffffffff0270aaf00800000000160014d85c2b71d0060b09c9886aeb815e50991dda124d00e1f5050000000016001400aea9a2e5f0f876a588df5546e8742d1d87008f000000000000000000";

    /// Test QrScanner parses BBQr with FileType::Psbt correctly
    /// This would have caught the "Invalid UTF-8" bug
    #[test]
    fn test_qr_scanner_single_part_bbqr_psbt() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let psbt_bytes = hex::decode(TEST_PSBT_HEX).unwrap();

        // create BBQr with FileType::Psbt (binary, not text!)
        let split = Split::try_from_data(
            &psbt_bytes,
            FileType::Psbt,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 1,
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode PSBT as BBQr");

        let bbqr_string = &split.parts[0];

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(bbqr_string.clone()));

        assert!(result.is_ok(), "Scanner should succeed for PSBT BBQr: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Transaction(_)),
                    "PSBT should parse as Transaction (unsigned tx extracted), got: {:?}",
                    data
                );
            }
            ScanResult::InProgress { .. } => {
                panic!("Single-part BBQr should complete immediately");
            }
        }
    }

    /// Test QrScanner parses BBQr with FileType::Transaction correctly
    #[test]
    fn test_qr_scanner_single_part_bbqr_transaction() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        // extract unsigned transaction from PSBT
        let psbt_bytes = hex::decode(TEST_PSBT_HEX).unwrap();
        let crypto_psbt = cove_ur::CryptoPsbt::from_psbt_bytes(psbt_bytes).unwrap();
        let unsigned_tx = &crypto_psbt.psbt().unsigned_tx;
        let tx_bytes = bitcoin::consensus::serialize(unsigned_tx);

        // create BBQr with FileType::Transaction
        let split = Split::try_from_data(
            &tx_bytes,
            FileType::Transaction,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 1,
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode transaction as BBQr");

        let bbqr_string = &split.parts[0];

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(bbqr_string.clone()));

        assert!(result.is_ok(), "Scanner should succeed for Transaction BBQr: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, .. } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Transaction(_)),
                    "Transaction BBQr should parse as Transaction, got: {:?}",
                    data
                );
            }
            ScanResult::InProgress { .. } => {
                panic!("Single-part BBQr should complete immediately");
            }
        }
    }

    /// Test multi-part BBQr with FileType::Psbt
    #[test]
    fn test_qr_scanner_multi_part_bbqr_psbt() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let psbt_bytes = hex::decode(TEST_PSBT_HEX).unwrap();

        // force multi-part by using small QR version
        let split = Split::try_from_data(
            &psbt_bytes,
            FileType::Psbt,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 3,
                max_split_number: 10,
                min_version: Version::V01,
                max_version: Version::V05,
            },
        )
        .expect("should encode PSBT as multi-part BBQr");

        assert!(split.parts.len() > 1, "Should have multiple parts, got {}", split.parts.len());

        let scanner = QrScannerFFI::new();

        // scan all parts
        for (i, part) in split.parts.iter().enumerate() {
            let result = scanner.scan(StringOrData::String(part.clone()));
            assert!(result.is_ok(), "Part {} should scan successfully: {:?}", i, result);

            match result.unwrap() {
                ScanResult::InProgress {
                    progress: ScanProgress::Bbqr { scanned, total }, ..
                } => {
                    assert_eq!(scanned as usize, i + 1, "Scanned count should match");
                    assert_eq!(total as usize, split.parts.len(), "Total should match");
                }
                ScanResult::Complete { data, .. } => {
                    assert_eq!(i, split.parts.len() - 1, "Should only complete on last part");
                    assert!(
                        matches!(data, crate::multi_format::MultiFormat::Transaction(_)),
                        "Multi-part PSBT BBQr should parse as Transaction, got: {:?}",
                        data
                    );
                }
                _ => panic!("Unexpected result for part {}", i),
            }
        }
    }

    /// Test single-part UR completes immediately
    #[test]
    fn test_single_part_ur() {
        // known single-part UR (crypto-output) from multi_format tests
        let ur_string = "UR:CRYPTO-OUTPUT/TAADMWTAADDLOSAOWKAXHDCLAXNSRSIMBNDRBNFTDEJSAXADLSMTWNDSAOWPLBIHFLSBEMLGMWCTDWDSFTFLDACPREAAHDCXMOCXBYKEGWNBDYADGHEMPYCFHGEYRYCATDTIWTWTLBGTSGPEGYECBDDARFHTFNLFAHTAADEHOEADAEAOAEAMTAADDYOTADLNCSGHYKAEYKAEYKAOCYGHENTSDKAXAXAYCYBGKBNBVAASIHFWGAGDEOESCLCFPSPY";

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(ur_string.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, haptic } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should parse as HardwareExport"
                );
                assert_eq!(haptic, HapticFeedback::Success);
            }
            ScanResult::InProgress { .. } => {
                panic!("Single-part UR should complete immediately");
            }
        }
    }

    /// Test SeedQR string format (numeric)
    #[test]
    fn test_seed_qr_string() {
        // from seed_qr.rs test: 12-word mnemonic as numeric indexes
        let seed_qr_string = "192402220235174306311124037817700641198012901210";

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::String(seed_qr_string.to_string()));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, haptic } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Mnemonic(_)),
                    "Should parse as Mnemonic"
                );
                assert_eq!(haptic, HapticFeedback::Success);
            }
            ScanResult::InProgress { .. } => {
                panic!("SeedQR string should complete immediately");
            }
        }
    }

    /// Test SeedQR binary data format (compact)
    #[test]
    fn test_seed_qr_binary_data() {
        // from seed_qr.rs test: 16 bytes of entropy for 12-word mnemonic
        let entropy = hex::decode("5bbd9d71a8ec7990831aff359d426545").unwrap();

        let scanner = QrScannerFFI::new();
        let result = scanner.scan(StringOrData::Data(entropy));

        assert!(result.is_ok(), "Scanner should succeed: {:?}", result);

        match result.unwrap() {
            ScanResult::Complete { data, haptic } => {
                assert!(
                    matches!(data, crate::multi_format::MultiFormat::Mnemonic(_)),
                    "Should parse as Mnemonic"
                );
                assert_eq!(haptic, HapticFeedback::Success);
            }
            ScanResult::InProgress { .. } => {
                panic!("SeedQR binary should complete immediately");
            }
        }
    }

    /// Test multi-part BBQr UnicodeText scans to completion
    #[test]
    fn test_multi_part_bbqr_unicode_text_completion() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // force multi-part
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

        let scanner = QrScannerFFI::new();

        // scan all parts
        for (i, part) in split.parts.iter().enumerate() {
            let result = scanner.scan(StringOrData::String(part.clone())).unwrap();

            if i < split.parts.len() - 1 {
                assert!(
                    matches!(result, ScanResult::InProgress { .. }),
                    "Should be in progress for part {i}"
                );
            } else {
                match result {
                    ScanResult::Complete { data, haptic } => {
                        assert!(
                            matches!(data, crate::multi_format::MultiFormat::Address(_)),
                            "Should parse as Address"
                        );
                        assert_eq!(haptic, HapticFeedback::Success);
                    }
                    _ => panic!("Last part should complete"),
                }
            }
        }
    }

    /// Test haptic feedback returns None for duplicate parts
    #[test]
    fn test_haptic_feedback_duplicate_part() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // force multi-part
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

        let scanner = QrScannerFFI::new();

        // scan first part
        let first_result = scanner.scan(StringOrData::String(split.parts[0].clone())).unwrap();
        match first_result {
            ScanResult::InProgress { haptic, .. } => {
                assert_eq!(
                    haptic,
                    HapticFeedback::Progress,
                    "First scan should have Progress haptic"
                );
            }
            _ => panic!("First scan should be in progress"),
        }

        // scan same part again - should return None haptic
        let duplicate_result = scanner.scan(StringOrData::String(split.parts[0].clone())).unwrap();
        match duplicate_result {
            ScanResult::InProgress { haptic, .. } => {
                assert_eq!(haptic, HapticFeedback::None, "Duplicate scan should have None haptic");
            }
            _ => panic!("Duplicate scan should still be in progress"),
        }
    }

    /// Test binary data to multi-part scan returns RequiresStringData error
    #[test]
    fn test_binary_data_to_multi_part_scan_error() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // force multi-part BBQr
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

        let scanner = QrScannerFFI::new();

        // start multi-part scan
        let _ = scanner.scan(StringOrData::String(split.parts[0].clone())).unwrap();

        // try to add binary data - should fail
        let binary_data = vec![0x01, 0x02, 0x03];
        let result = scanner.scan(StringOrData::Data(binary_data));

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), MultiQrError::RequiresStringData),
            "Should return RequiresStringData error"
        );
    }

    /// Test BBQr parts can be scanned out of order
    #[test]
    fn test_out_of_order_bbqr_parts() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";

        // force multi-part
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

        assert!(split.parts.len() >= 3, "Need at least 3 parts for this test");

        let scanner = QrScannerFFI::new();

        // scan in reverse order
        let mut reversed_parts: Vec<_> = split.parts.to_vec();
        reversed_parts.reverse();

        let mut completed = false;
        for part in &reversed_parts {
            let result = scanner.scan(StringOrData::String(part.clone())).unwrap();

            match result {
                ScanResult::InProgress { .. } => {}
                ScanResult::Complete { data, .. } => {
                    assert!(
                        matches!(data, crate::multi_format::MultiFormat::Address(_)),
                        "Should parse as Address"
                    );
                    completed = true;
                    break;
                }
            }
        }

        assert!(completed, "Should complete after scanning all parts in reverse order");
    }
}
