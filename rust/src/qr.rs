use std::sync::Arc;

use bbqr::{
    continuous_join::{ContinuousJoinResult, ContinuousJoiner},
    header::Header,
    join::Joined,
};
use bip39::Language;
use parking_lot::Mutex;

use crate::{
    ffi::scan_result_data::FfiScanResultData,
    seed_qr::{SeedQr, SeedQrError},
};

#[derive(uniffi::Object)]
pub enum MultiQr {
    SeedQr(SeedQr),
    Single(String),
    Bbqr(Header, Arc<Mutex<ContinuousJoiner>>),
}

#[derive(Debug, uniffi::Object)]
pub struct BbqrJoinResult(ContinuousJoinResult);

#[derive(Debug, uniffi::Object)]
pub struct BbqrJoined(Joined);

type Error = MultiQrError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
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

    #[error("BBQr did not container seed words")]
    BbqrDidNotContainSeedWords,

    #[error(transparent)]
    InvalidSeedQr(#[from] SeedQrError),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MultiQrScanResult {
    SeedQr(Arc<SeedQr>),
    Single(String),
    CompletedBBqr(Arc<BbqrJoined>),
    InProgressBBqr(u32),
}

#[uniffi::export]
impl MultiQr {
    #[uniffi::constructor]
    pub fn try_new(qr: FfiScanResultData) -> Result<Self, Error> {
        type R = FfiScanResultData;
        match qr {
            R::String(qr) => Ok(Self::new_from_string(qr)),
            R::Data(data) => Self::try_new_from_data(data),
        }
    }

    #[uniffi::constructor]
    pub fn new_from_string(qr: String) -> Self {
        // try to parse bbqr
        if let Ok(header) = bbqr::header::Header::try_from_str(&qr) {
            let mut continuous_joiner = bbqr::continuous_join::ContinuousJoiner::new();
            continuous_joiner
                .add_part(qr)
                .expect("already checked that header is valid");

            let continuous_joiner = Arc::new(Mutex::new(continuous_joiner));
            return Self::Bbqr(header, continuous_joiner);
        }

        // try to parse standard seed qr
        if let Ok(seed_qr) = SeedQr::try_from_str(&qr) {
            return Self::SeedQr(seed_qr);
        }

        // default to single qr
        Self::Single(qr)
    }

    #[uniffi::constructor]
    pub fn try_new_from_data(data: Vec<u8>) -> Result<Self, Error> {
        let seed_qr = SeedQr::try_from_data(data)?;
        Ok(Self::SeedQr(seed_qr))
    }

    #[uniffi::method]
    pub fn handle_scan_result(
        &self,
        qr: FfiScanResultData,
    ) -> Result<MultiQrScanResult, MultiQrError> {
        type R = FfiScanResultData;

        let result = match (self, qr) {
            (Self::SeedQr(seed_qr), _) => MultiQrScanResult::SeedQr(Arc::new(seed_qr.clone())),
            (Self::Single(_), R::String(qr)) => MultiQrScanResult::Single(qr),
            (Self::Bbqr(_, joiner), R::String(qr)) => {
                let join_result = joiner
                    .lock()
                    .add_part(qr)
                    .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

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
            (Self::Bbqr(_, _), FfiScanResultData::Data(_vec)) => {
                return Err(MultiQrError::CannotAddBinaryDataToBbqr)
            }

            (Self::Single(_), R::Data(_)) => return Err(MultiQrError::CannotAddPartToSingleQr),
        };

        Ok(result)
    }

    #[uniffi::method]
    pub fn is_single(&self) -> bool {
        matches!(self, MultiQr::Single(_))
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
    pub fn add_part(&self, qr: String) -> Result<BbqrJoinResult, MultiQrError> {
        match self {
            MultiQr::Bbqr(_, continuous_joiner) => {
                let join_result = continuous_joiner
                    .lock()
                    .add_part(qr)
                    .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

                Ok(BbqrJoinResult(join_result))
            }

            // error
            MultiQr::SeedQr(_) => Err(MultiQrError::CannotAddPartToSeedQr),
            MultiQr::Single(_) => Err(MultiQrError::CannotAddPartToSingleQr),
        }
    }

    #[uniffi::method]
    pub fn total_parts(&self) -> u32 {
        match self {
            MultiQr::Bbqr(header, _) => header.num_parts as u32,
            MultiQr::SeedQr(_) => 1,
            MultiQr::Single(_) => 1,
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
        let word_list = Language::English.word_list();

        self.0
            .data
            .iter()
            .map(|word| word.to_string().to_ascii_lowercase())
            .map(|word| word_list.iter().find(|w| w.starts_with(&word)))
            .map(|word| {
                let word = word.ok_or(MultiQrError::BbqrDidNotContainSeedWords)?;
                Ok(word.to_string())
            })
            .collect()
    }
}
