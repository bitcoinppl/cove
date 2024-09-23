use std::sync::Arc;

use bbqr::{
    continuous_join::{ContinuousJoinResult, ContinuousJoiner},
    header::Header,
};
use parking_lot::Mutex;

#[derive(uniffi::Object)]
pub enum MultiQr {
    Single(String),
    Bbqr(Header, Arc<Mutex<ContinuousJoiner>>),
}

#[derive(Debug, uniffi::Object)]
pub struct BbqrJoinResult(ContinuousJoinResult);

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MultiQrError {
    #[error("Cannot add part to single QR")]
    CannotAddPartToSingleQr,

    #[error("BBQr Parse error: {0}")]
    ParseError(String),

    #[error("Invalid UTF-8")]
    InvalidUtf8,

    #[error("Final result not yet available, joining not complete")]
    NotYetAvailable,
}

#[uniffi::export]
impl MultiQr {
    #[uniffi::constructor]
    pub fn new(qr: String) -> Self {
        if let Ok(header) = bbqr::header::Header::try_from_str(&qr) {
            let mut continuous_joiner = bbqr::continuous_join::ContinuousJoiner::new();
            continuous_joiner
                .add_part(qr)
                .expect("already checked that header is valid");

            let continuous_joiner = Arc::new(Mutex::new(continuous_joiner));
            return Self::Bbqr(header, continuous_joiner);
        }

        Self::Single(qr)
    }

    #[uniffi::method]
    pub fn is_single(&self) -> bool {
        matches!(self, MultiQr::Single(_))
    }

    #[uniffi::method]
    pub fn add_part(&self, qr: String) -> Result<BbqrJoinResult, MultiQrError> {
        match self {
            MultiQr::Single(_) => Err(MultiQrError::CannotAddPartToSingleQr),
            MultiQr::Bbqr(_, continuous_joiner) => {
                let join_result = continuous_joiner
                    .lock()
                    .add_part(qr)
                    .map_err(|e| MultiQrError::ParseError(e.to_string()))?;

                Ok(BbqrJoinResult(join_result))
            }
        }
    }

    #[uniffi::method]
    pub fn total_parts(&self) -> u32 {
        match self {
            MultiQr::Single(_) => 1,
            MultiQr::Bbqr(header, _) => header.num_parts as u32,
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
