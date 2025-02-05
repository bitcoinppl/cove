use bitcoin::base64::{prelude::BASE64_URL_SAFE, Engine as _};
use winnow::{
    token::{take_until, take_while},
    Parser as _, Result as WinnowResult,
};

use crate::transaction::ffi::BitcoinTransaction;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Object)]
pub struct PushTx {
    pub txn: BitcoinTransaction,
}

#[derive(Debug, Copy, Clone, thiserror::Error)]
pub enum PushTxError {
    #[error("The string is not a valid pushtx")]
    InvalidPushTx,

    #[error("The transaction is not a valid base64 string")]
    InvalidBase64,

    #[error("The transaction is not a valid transaction")]
    InvalidTransaction,
}

pub type Error = PushTxError;

type Result<T, E = Error> = std::result::Result<T, E>;

impl PushTx {
    pub fn try_from_str(string: &str) -> Result<Self> {
        let mut string = string.trim();
        let base64 = extract_tx(&mut string).map_err(|_| PushTxError::InvalidPushTx)?;

        let txn_bytes: Vec<u8> = BASE64_URL_SAFE
            .decode(base64.as_bytes())
            .map_err(|_| PushTxError::InvalidBase64)?;

        let txn = BitcoinTransaction::try_from_data(&txn_bytes)
            .map_err(|_| PushTxError::InvalidTransaction)?;

        Ok(Self { txn })
    }
}

fn extract_tx<'s>(input: &mut &'s str) -> WinnowResult<&'s str> {
    // skip until we find "pushtx#t="
    let _ = take_until(1.., "pushtx#t=").parse_next(input)?;

    // skip "pushtx#t="
    let _ = "pushtx#t=".parse_next(input)?;

    // Take everything until the next &
    take_while(0.., |c| c != '&').parse_next(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_pushtx() {
        let push_tx = "\u{04}coldcard.com/pushtx#t=AQAAAAABAay4nldPI1derSdI1ht84RFrTeJyYf5n63mfXSbOFyvcAAAAAAD9____AvYwAAAAAAAAFgAUhQSKvG3Ilu3l-nFpwGasJJ7EW7Ay8QEAAAAAABYAFPoi-bNQDvD-241n9abk64iaIsgQAkcwRAIgKEdWCbN11wytyTrPsvjG_5ukZ3QklTq-KVB_2sEpkzsCIBDmeh3QzotQhf44YDuwmR6-90At-92P3208jBEw7aXIASEDWeWFXdH0sE9jMNJDi1erWKRHY0QIJoLjZjO04q4b3ZIQ-RoA&c=uBUeldpP9dY&n=XTN";
        let tx = PushTx::try_from_str(push_tx);
        assert!(tx.is_ok());
    }

    #[test]
    fn test_parses_pushtx_without_extra_data() {
        let push_tx = "coldcard.com/pushtx#t=AQAAAAABAay4nldPI1derSdI1ht84RFrTeJyYf5n63mfXSbOFyvcAAAAAAD9____AvYwAAAAAAAAFgAUhQSKvG3Ilu3l-nFpwGasJJ7EW7Ay8QEAAAAAABYAFPoi-bNQDvD-241n9abk64iaIsgQAkcwRAIgKEdWCbN11wytyTrPsvjG_5ukZ3QklTq-KVB_2sEpkzsCIBDmeh3QzotQhf44YDuwmR6-90At-92P3208jBEw7aXIASEDWeWFXdH0sE9jMNJDi1erWKRHY0QIJoLjZjO04q4b3ZIQ-RoA";
        let tx = PushTx::try_from_str(push_tx);
        assert!(tx.is_ok());
    }
}
