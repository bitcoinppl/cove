//! QR code scanning state machine for single and multi-part QR codes.
//!
//! [`MultiQr`] manages the scanning process for potentially multi-part QR codes like
//! BBQR (animated) and UR (fountain-coded). It maintains decoder state and tracks
//! progress until all parts are received.
//!
//! This represents the "how" - how to reassemble animated QR sequences. Once scanning
//! completes, the result is converted to [`crate::multi_format::MultiFormat`].

use core::str;
use std::sync::Arc;

use bbqr::{
    continuous_join::{ContinuousJoinResult, ContinuousJoiner},
    header::Header,
    join::Joined,
};
use bip39::{Language, Mnemonic};
use cove_ur::Ur;
use foundation_ur::Decoder as UrDecoder;
use parking_lot::Mutex;
use tracing::{debug, warn};

use crate::{
    mnemonic::{ParseMnemonic as _, WordAccess as _},
    multi_format::StringOrData,
    seed_qr::{SeedQr, SeedQrError},
    ur::UrType,
};
use cove_util::result_ext::ResultExt as _;

#[derive(uniffi::Object)]
#[allow(dead_code)]
pub enum MultiQr {
    SeedQr(SeedQr),
    Single(String),
    Bbqr(Header, Arc<Mutex<ContinuousJoiner>>),
    Ur(Arc<Mutex<UrDecoder>>),
}

#[derive(Debug, uniffi::Object)]
pub struct BbqrJoinResult(ContinuousJoinResult);

#[derive(Debug, uniffi::Object)]
pub struct BbqrJoined(Joined);

type Error = MultiQrError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi::export(Display)]
pub enum MultiQrError {
    #[error("Cannot add part to single QR")]
    CannotAddPartToSingleQr,

    #[error("Cannot add part to seed QR")]
    CannotAddPartToSeedQr,

    #[error("BBQr Parse error: {0}")]
    ParseError(String),

    #[error("Invalid UTF-8")]
    InvalidUtf8,

    #[error("Final result not yet available, joining not complete")]
    NotYetAvailable,

    #[error("Cannot add binary data to BBQR")]
    CannotAddBinaryDataToBbqr,

    #[error("BBQR did not contain seed words, found: {0}")]
    BbqrDidNotContainSeedWords(String),

    #[error(transparent)]
    InvalidSeedQr(#[from] SeedQrError),

    #[error("Invalid plain text seed QR")]
    InvalidPlainTextQr(String),

    #[error(transparent)]
    Ur(#[from] cove_ur::UrError),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MultiQrScanResult {
    SeedQr(Arc<SeedQr>),
    Single(String),
    CompletedBBqr(Arc<BbqrJoined>),
    InProgressBBqr(u32),
    CompletedUr(crate::multi_format::MultiFormat),
    InProgressUr(f64),
}

/// Result of a QR scan - either complete with parsed data or in progress
#[derive(Debug, Clone, uniffi::Enum)]
pub enum ScanResult {
    /// Scan complete - here's the parsed data
    Complete(crate::multi_format::MultiFormat),
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

#[uniffi::export]
impl MultiQr {
    #[uniffi::constructor]
    pub fn try_new(qr: StringOrData) -> Result<Self, Error> {
        type R = StringOrData;
        match qr {
            R::String(qr) => Ok(Self::new_from_string(qr)),
            R::Data(data) => Self::try_new_from_data(data),
        }
    }

    #[uniffi::constructor]
    pub fn new_from_string(qr: String) -> Self {
        // try to parse UR (check before BBQR since UR also uses uppercase prefix)
        if qr.to_lowercase().starts_with("ur:") {
            debug!("detected UR prefix, attempting to parse...");
            match Ur::parse(&qr) {
                Ok(ur) => {
                    // check if single-part or multi-part UR
                    match ur.to_foundation_ur() {
                        foundation_ur::UR::SinglePart { .. }
                        | foundation_ur::UR::SinglePartDeserialized { .. } => {
                            // single-part UR - handle via MultiFormat directly
                            debug!("Single-part UR, treating as Single");
                            return Self::Single(qr);
                        }
                        foundation_ur::UR::MultiPart { .. }
                        | foundation_ur::UR::MultiPartDeserialized { .. } => {
                            // multi-part UR - use decoder
                            debug!("Multi-part UR, using decoder");
                            let mut decoder = UrDecoder::default();
                            let _ = decoder.receive(ur.to_foundation_ur());
                            return Self::Ur(Arc::new(Mutex::new(decoder)));
                        }
                    }
                }
                Err(e) => {
                    warn!("UR parse failed: {:?}", e);
                }
            }
        }

        // try to parse bbqr
        if let Ok(header) = bbqr::header::Header::try_from_str(&qr) {
            let mut continuous_joiner = bbqr::continuous_join::ContinuousJoiner::new();
            let join_result = continuous_joiner
                .add_part(qr.clone())
                .expect("already checked that header is valid");

            // check if single-part BBQR completed immediately
            if let ContinuousJoinResult::Complete(result) = join_result {
                if let Ok(data_string) = String::from_utf8(result.data) {
                    return Self::Single(data_string);
                }
            }

            let continuous_joiner = Arc::new(Mutex::new(continuous_joiner));
            return Self::Bbqr(header, continuous_joiner);
        }

        // try to parse standard seed qr
        if let Ok(seed_qr) = SeedQr::try_from_str(&qr) {
            return Self::SeedQr(seed_qr);
        }

        Self::Single(qr)
    }

    #[uniffi::constructor]
    pub fn try_new_from_data(data: Vec<u8>) -> Result<Self, Error> {
        let seed_qr = SeedQr::try_from_data(&data)?;
        Ok(Self::SeedQr(seed_qr))
    }

    #[uniffi::method]
    pub fn handle_scan_result(&self, qr: StringOrData) -> Result<MultiQrScanResult, MultiQrError> {
        type R = StringOrData;

        let result = match (self, qr) {
            (Self::SeedQr(seed_qr), _) => MultiQrScanResult::SeedQr(Arc::new(seed_qr.clone())),
            (Self::Single(stored), R::String(_)) => MultiQrScanResult::Single(stored.clone()),

            // UR handling
            (Self::Ur(decoder), R::String(qr)) => {
                use cove_ur::UrError;

                let ur = Ur::parse(&qr)?;

                let mut decoder = decoder.lock();
                decoder
                    .receive(ur.to_foundation_ur())
                    .map_err(|e| UrError::UrParseError(e.to_string()))?;

                if decoder.is_complete() {
                    let message = decoder
                        .message()
                        .map_err(|e| UrError::UrParseError(e.to_string()))?
                        .ok_or_else(|| UrError::UrParseError("No message".into()))?;
                    let ur_type_str = decoder.ur_type().unwrap_or("bytes");
                    debug!("UR complete, type: {}, message len: {}", ur_type_str, message.len());

                    let ur_type = UrType::from_str(ur_type_str);
                    debug!("Parsed UR type: {:?}", ur_type);

                    // Convert to MultiFormat directly
                    let multi_format =
                        crate::multi_format::MultiFormat::try_from_ur_payload(message, &ur_type)
                            .map_err(|e| {
                                warn!("Failed to parse UR payload: {}", e);
                                MultiQrError::ParseError(format!(
                                    "Failed to parse UR payload: {}",
                                    e
                                ))
                            })?;

                    MultiQrScanResult::CompletedUr(multi_format)
                } else {
                    let progress = decoder.estimated_percent_complete();
                    MultiQrScanResult::InProgressUr(progress)
                }
            }

            (Self::Bbqr(_, joiner), R::String(qr)) => {
                let join_result =
                    joiner.lock().add_part(qr).map_err_str(MultiQrError::ParseError)?;

                match join_result {
                    ContinuousJoinResult::Complete(result) => {
                        MultiQrScanResult::CompletedBBqr(Arc::new(BbqrJoined(result)))
                    }

                    ContinuousJoinResult::InProgress { parts_left } => {
                        MultiQrScanResult::InProgressBBqr(parts_left as u32)
                    }

                    ContinuousJoinResult::NotStarted => panic!("not started, not possible"),
                }
            }

            // errors
            (Self::Ur(_), StringOrData::Data(_)) => {
                use cove_ur::UrError;
                return Err(UrError::UrParseError("UR requires string data".into()).into());
            }

            (Self::Bbqr(_, _), StringOrData::Data(_vec)) => {
                return Err(MultiQrError::CannotAddBinaryDataToBbqr);
            }

            (Self::Single(_), R::Data(_)) => return Err(MultiQrError::CannotAddPartToSingleQr),
        };

        Ok(result)
    }

    #[uniffi::method]
    pub fn get_grouped_words(
        &self,
        qr: StringOrData,
        groups_of: u8,
    ) -> Result<Option<Vec<Vec<String>>>, MultiQrError> {
        let words: Option<Vec<Vec<String>>> = match self.handle_scan_result(qr)? {
            MultiQrScanResult::SeedQr(seed_qr) => {
                let mnemonic = seed_qr.mnemonic();
                let grouped = mnemonic.grouped_plain_words_of(groups_of as usize);

                Some(grouped)
            }

            MultiQrScanResult::Single(qr) => {
                let bip39 =
                    Mnemonic::parse_in(Language::English, &qr).or_else(|_| qr.parse_mnemonic());

                let words = bip39
                    .map_err(|_| MultiQrError::InvalidPlainTextQr(qr))?
                    .grouped_plain_words_of(groups_of as usize);

                Some(words)
            }

            MultiQrScanResult::CompletedBBqr(joined) => {
                let words = joined.get_grouped_words(groups_of)?;
                Some(words)
            }

            MultiQrScanResult::InProgressBBqr(_) => None,

            MultiQrScanResult::CompletedUr(_ur_result) => {
                // UR results are typically PSBTs or keys, not seed words
                None
            }

            MultiQrScanResult::InProgressUr(_) => None,
        };

        Ok(words)
    }

    #[uniffi::method]
    pub fn is_seed_qr(&self) -> bool {
        matches!(self, MultiQr::SeedQr(_))
    }

    #[uniffi::method]
    pub fn is_bbqr(&self) -> bool {
        matches!(self, MultiQr::Bbqr(_, _))
    }

    #[uniffi::method]
    pub fn is_ur(&self) -> bool {
        matches!(self, MultiQr::Ur(_))
    }

    #[uniffi::method]
    pub fn add_part(&self, qr: String) -> Result<BbqrJoinResult, MultiQrError> {
        match self {
            MultiQr::Bbqr(_, continuous_joiner) => {
                let join_result =
                    continuous_joiner.lock().add_part(qr).map_err_str(MultiQrError::ParseError)?;

                Ok(BbqrJoinResult(join_result))
            }

            // error
            MultiQr::SeedQr(_) => Err(MultiQrError::CannotAddPartToSeedQr),
            MultiQr::Single(_) => Err(MultiQrError::CannotAddPartToSingleQr),
            MultiQr::Ur(_) => {
                use cove_ur::UrError;
                Err(UrError::UrParseError("Use handle_scan_result for UR parts".into()).into())
            }
        }
    }

    #[uniffi::method]
    pub fn total_parts(&self) -> u32 {
        match self {
            MultiQr::Bbqr(header, _) => header.num_parts as u32,
            MultiQr::SeedQr(_) => 1,
            MultiQr::Single(_) => 1,
            MultiQr::Ur(_) => {
                // UR uses fountain codes - parts are not fixed
                // return 0 to indicate unknown/variable
                0
            }
        }
    }
}

#[uniffi::export]
impl BbqrJoinResult {
    pub fn is_complete(&self) -> bool {
        matches!(self.0, ContinuousJoinResult::Complete(_))
    }

    pub fn final_result(&self) -> Result<String, MultiQrError> {
        match &self.0 {
            ContinuousJoinResult::Complete(result) => {
                let data = result.data.clone();
                let string = String::from_utf8(data).map_err(|_| MultiQrError::InvalidUtf8)?;
                Ok(string)
            }
            ContinuousJoinResult::InProgress { .. } => Err(MultiQrError::NotYetAvailable),
            ContinuousJoinResult::NotStarted => Err(MultiQrError::NotYetAvailable),
        }
    }

    pub fn parts_left(&self) -> u32 {
        match self.0 {
            ContinuousJoinResult::Complete(_) => 0,
            ContinuousJoinResult::InProgress { parts_left } => parts_left as u32,
            ContinuousJoinResult::NotStarted => panic!("not started, not possible"),
        }
    }
}

#[uniffi::export]
impl BbqrJoined {
    pub fn get_seed_words(&self) -> Result<Vec<String>, Error> {
        let words_str = str::from_utf8(&self.0.data).map_err(|_| MultiQrError::InvalidUtf8)?;
        let mnemonic = words_str.parse_mnemonic().map_err_str(MultiQrError::ParseError)?;

        Ok(mnemonic.words().map(ToString::to_string).collect())
    }

    pub fn get_grouped_words(&self, chunks: u8) -> Result<Vec<Vec<String>>, Error> {
        let words = self.get_seed_words()?;
        let grouped = words.chunks(chunks as usize).map(|chunk| chunk.to_vec()).collect();

        Ok(grouped)
    }

    pub fn final_result(&self) -> Result<String, Error> {
        let data = self.0.data.clone();
        String::from_utf8(data).map_err(|_| MultiQrError::InvalidUtf8)
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
        Self {
            state: Mutex::new(None),
        }
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
                Ok((None, ScanResult::Complete(multi_format)))
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
                    foundation_ur::UR::SinglePart { .. }
                    | foundation_ur::UR::SinglePartDeserialized { .. } => {
                        // single-part UR - convert directly to MultiFormat
                        debug!("Single-part UR, converting to MultiFormat");
                        let multi_format = MultiFormat::try_from_string(&qr)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                        return Ok((None, ScanResult::Complete(multi_format)));
                    }
                    foundation_ur::UR::MultiPart { .. }
                    | foundation_ur::UR::MultiPartDeserialized { .. } => {
                        // multi-part UR - create decoder and return in-progress
                        debug!("Multi-part UR, using decoder");
                        let mut decoder = UrDecoder::default();
                        let _ = decoder.receive(ur.to_foundation_ur());
                        let percentage = decoder.estimated_percent_complete();
                        let multi_qr = MultiQr::Ur(Arc::new(Mutex::new(decoder)));
                        return Ok((
                            Some(multi_qr),
                            ScanResult::InProgress(ScanProgress::Ur { percentage }),
                        ));
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
                    let data_string = String::from_utf8(result.data)
                        .map_err(|_| MultiQrError::InvalidUtf8)?;
                    let multi_format =
                        crate::multi_format::MultiFormat::try_from_string(&data_string)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                    return Ok((None, ScanResult::Complete(multi_format)));
                }
                ContinuousJoinResult::InProgress { parts_left } => {
                    // multi-part BBQR - return in-progress
                    let total = header.num_parts as u32;
                    let scanned = total - parts_left as u32;
                    let multi_qr =
                        MultiQr::Bbqr(header, Arc::new(Mutex::new(continuous_joiner)));
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
            return Ok((None, ScanResult::Complete(multi_format)));
        }

        // plain string - try to convert to MultiFormat directly
        let multi_format = crate::multi_format::MultiFormat::try_from_string(&qr)
            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
        Ok((None, ScanResult::Complete(multi_format)))
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
                        Ok(ScanResult::Complete(multi_format))
                    }
                    ContinuousJoinResult::InProgress { parts_left } => {
                        let total = header.num_parts as u32;
                        let scanned = total - parts_left as u32;
                        Ok(ScanResult::InProgress(ScanProgress::Bbqr {
                            scanned,
                            total,
                        }))
                    }
                    ContinuousJoinResult::NotStarted => {
                        Err(MultiQrError::ParseError("BBQR not started".into()))
                    }
                }
            }

            (MultiQr::Ur(decoder), StringOrData::String(qr_string)) => {
                use cove_ur::UrError;

                let ur = Ur::parse(&qr_string)?;
                let mut decoder = decoder.lock();
                decoder
                    .receive(ur.to_foundation_ur())
                    .map_err(|e| UrError::UrParseError(e.to_string()))?;

                if decoder.is_complete() {
                    let message = decoder
                        .message()
                        .map_err(|e| UrError::UrParseError(e.to_string()))?
                        .ok_or_else(|| UrError::UrParseError("No message".into()))?;
                    let ur_type_str = decoder.ur_type().unwrap_or("bytes");
                    debug!(
                        "UR complete, type: {}, message len: {}",
                        ur_type_str,
                        message.len()
                    );

                    let ur_type = crate::ur::UrType::from_str(ur_type_str);
                    let multi_format =
                        crate::multi_format::MultiFormat::try_from_ur_payload(message, &ur_type)
                            .map_err(|e| MultiQrError::ParseError(e.to_string()))?;
                    Ok(ScanResult::Complete(multi_format))
                } else {
                    let percentage = decoder.estimated_percent_complete();
                    Ok(ScanResult::InProgress(ScanProgress::Ur { percentage }))
                }
            }

            // error cases - can't add parts to single-part QRs
            (MultiQr::Single(_), _) => Err(MultiQrError::CannotAddPartToSingleQr),
            (MultiQr::SeedQr(_), _) => Err(MultiQrError::CannotAddPartToSeedQr),
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

    // test vector from foundation-ur test suite
    const SINGLE_PART_BYTES_UR: &str =
        "ur:bytes/hdcxlkahssqzwfvslofzoxwkrewngotktbmwjkwdvlnehsmtkgkhenpmhsmtpctnesspyahgs";

    #[test]
    fn test_single_part_ur_treated_as_single() {
        // single-part UR should be treated as Single (not Ur) for direct handling
        let qr = MultiQr::new_from_string(SINGLE_PART_BYTES_UR.to_string());
        assert!(!qr.is_ur()); // single-part URs are NOT treated as Ur variant
        assert!(!qr.is_bbqr());
        assert!(!qr.is_seed_qr());
        assert!(matches!(qr, MultiQr::Single(_)));
    }

    #[test]
    fn test_ur_vs_other_formats() {
        // single-part UR is treated as Single for direct MultiFormat handling
        let ur = MultiQr::new_from_string("ur:bytes/test".to_string());
        assert!(matches!(ur, MultiQr::Single(_)));

        // non-UR should not be detected as UR
        let single = MultiQr::new_from_string("some random text".to_string());
        assert!(!single.is_ur());
        assert!(matches!(single, MultiQr::Single(_)));
    }

    #[test]
    fn test_single_part_ur_total_parts() {
        // single-part UR is treated as Single, so total_parts should be 1
        let qr = MultiQr::new_from_string(SINGLE_PART_BYTES_UR.to_string());
        assert_eq!(qr.total_parts(), 1);
    }

    #[test]
    fn test_single_part_ur_handle_scan_result() {
        // single-part UR treated as Single returns Single scan result
        let qr = MultiQr::new_from_string(SINGLE_PART_BYTES_UR.to_string());
        let result = qr.handle_scan_result(StringOrData::String(SINGLE_PART_BYTES_UR.to_string()));

        // should succeed and return Single
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), MultiQrScanResult::Single(_)));
    }

    /// End-to-end test: Single-part BBQR should return decoded content, not raw string
    /// This tests the bug where handle_scan_result returns the input instead of stored content
    #[test]
    fn test_single_part_bbqr_returns_decoded_content() {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let original = "hello world test content";

        // create a single-part BBQR
        let split = Split::try_from_data(
            original.as_bytes(),
            FileType::UnicodeText,
            SplitOptions {
                encoding: Encoding::Zlib,
                min_split_number: 1,
                max_split_number: 1, // force single part
                min_version: Version::V01,
                max_version: Version::V40,
            },
        )
        .expect("should encode");

        assert_eq!(split.parts.len(), 1, "Should be single-part BBQR");
        let bbqr_string = &split.parts[0];
        assert!(bbqr_string.starts_with("B$"), "Should be valid BBQR format");

        // first scan creates MultiQr with decoded content
        let multi_qr = MultiQr::new_from_string(bbqr_string.clone());

        // verify it's treated as Single (single-part BBQR completes immediately)
        assert!(matches!(multi_qr, MultiQr::Single(_)));

        // handle_scan_result should return the decoded content
        let result = multi_qr.handle_scan_result(StringOrData::String(bbqr_string.clone()));
        assert!(result.is_ok());

        match result.unwrap() {
            MultiQrScanResult::Single(content) => {
                // BUG: Currently returns raw BBQR string instead of decoded content
                assert!(
                    !content.starts_with("B$"),
                    "Should return decoded content, not raw BBQR string. Got: {}",
                    content
                );
                assert_eq!(content, original, "Should return original decoded content");
            }
            _ => panic!("Expected Single result"),
        }
    }

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
            ScanResult::Complete(multi_format) => {
                // should be an Address
                assert!(
                    matches!(multi_format, crate::multi_format::MultiFormat::Address(_)),
                    "Should be Address, got: {:?}",
                    multi_format
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
            ScanResult::Complete(multi_format) => {
                assert!(
                    matches!(multi_format, crate::multi_format::MultiFormat::Address(_)),
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
            ScanResult::Complete(multi_format) => {
                // should be a HardwareExport containing the xpub
                assert!(
                    matches!(multi_format, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should be HardwareExport, got: {:?}",
                    multi_format
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
            ScanResult::Complete(multi_format) => {
                assert!(
                    matches!(multi_format, crate::multi_format::MultiFormat::HardwareExport(_)),
                    "Should be HardwareExport"
                );
            }
            ScanResult::InProgress(_) => {
                panic!("Plain xpub should complete immediately");
            }
        }
    }
}
