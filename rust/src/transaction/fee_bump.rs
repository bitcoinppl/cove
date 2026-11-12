//! Replacing a pending transaction with one that pays a higher fee (BIP 125 RBF)
//!
//! The wallet actor decides whether a transaction can be fee bumped and builds the replacement.
//! This module holds the rules both depend on, so the eligibility checks, the fee floor and the
//! errors shown to the user stay in one place

use bdk_wallet::error::{BuildFeeBumpError, CreateTxError};
use bitcoin::{Amount, FeeRate as BdkFeeRate, Txid, Weight};

use crate::manager::wallet_manager::WalletManagerError;

/// Bitcoin Core's `-incrementalrelayfee` default before 29.1, 1 sat/vB, in sat/kwu
///
/// A replacement must pay for its own relay at this rate on top of the fees it replaces
/// (BIP 125 rule 4). Later Core releases lower the default to 0.1 sat/vB, but many nodes
/// still run the older one, so a replacement that clears this rate relays everywhere
const INCREMENTAL_RELAY_SAT_PER_KWU: u64 = 250;

/// Fee rates offered for a replacement are rounded up to this step, 0.1 sat/vB
///
/// A minimum such as 3.004 sat/vB would show as 3.00 and fail when typed back in, so the
/// floor is rounded up to a rate the fee screens display exactly
const FEE_RATE_STEP_SAT_PER_KWU: u64 = 25;

/// Why a transaction can't be fee bumped
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error, uniffi::Enum)]
pub enum FeeBumpBlocker {
    #[error("the transaction is already confirmed")]
    Confirmed,

    /// Another transaction spending the same coins took its place
    #[error("the transaction was already replaced")]
    Replaced,

    /// None of its inputs belong to this wallet, such as a payment it received
    #[error("this wallet did not send the transaction")]
    NotSentByWallet,

    /// The wallet holds no keys, so it could not sign a replacement
    #[error("a watch-only wallet can't sign a replacement")]
    WatchOnly,

    /// Some inputs belong to another wallet, as in a payjoin, so this wallet can't sign them
    #[error("the transaction spends coins from another wallet")]
    ForeignInputs,

    /// No input signals BIP 125, so the wallet won't replace it
    #[error("the transaction does not allow replacement")]
    NotReplaceable,

    /// It pays more than one address outside the wallet, which the send screens can't show
    #[error("the transaction pays more than one recipient")]
    MultipleRecipients,

    /// A later pending transaction spends one of its outputs and would be dropped with it
    #[error("a later pending transaction spends from this one")]
    PendingDescendant,

    /// A previous output it spends is missing from the wallet, so its fee is unknown
    #[error("the transaction's fee is unknown")]
    UnknownFee,
}

/// Why a replacement could not be built
///
/// Crosses the FFI as a flat error: the frontend gets the variant and its message
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FeeBumpError {
    #[error("transaction not found")]
    NotFound,

    #[error("can't send until the initial scan completes")]
    InitialScanIncomplete,

    #[error("can't speed up: {0}")]
    Blocked(#[from] FeeBumpBlocker),

    /// The requested rate is under the floor a node needs to accept the replacement
    #[error("fee rate is too low to replace the transaction, minimum {minimum} sat/vB",
        minimum = sat_per_vb(*.minimum))]
    FeeRateTooLow { minimum: BdkFeeRate },

    /// The wallet has no confirmed coin left to pay the higher fee
    #[error(
        "not enough confirmed funds to pay the higher fee, needed {needed}, available {available}"
    )]
    InsufficientFunds { needed: Amount, available: Amount },

    #[error("unable to build the replacement: {0}")]
    Build(String),
}

impl FeeBumpBlocker {
    /// A sentence explaining the blocker to the user
    pub fn user_message(&self) -> String {
        let message = match self {
            Self::Confirmed => "This transaction is already confirmed.",
            Self::Replaced => "This transaction was already replaced by another one.",
            Self::NotSentByWallet => "Only a transaction sent from this wallet can be sped up.",
            Self::WatchOnly => {
                "This is a watch-only wallet, so it can't sign the faster transaction that \
                 replaces this one."
            }
            Self::ForeignInputs => {
                "This transaction spends coins from another wallet, such as in a payjoin, so \
                 this wallet can't sign a replacement on its own."
            }
            Self::NotReplaceable => "This transaction was sent without allowing replacement.",
            Self::MultipleRecipients => {
                "This transaction pays more than one recipient, which speeding up doesn't \
                 support yet."
            }
            Self::PendingDescendant => {
                "A later pending transaction spends from this one. Speed up the later one instead."
            }
            Self::UnknownFee => "This wallet can't tell what fee the transaction paid.",
        };

        message.to_string()
    }
}

#[uniffi::export]
impl FeeBumpBlocker {
    #[uniffi::method(name = "userMessage")]
    fn ffi_user_message(&self) -> String {
        self.user_message()
    }
}

/// Shown when no confirmed coin is left to pay a replacement's higher fee
pub const INSUFFICIENT_FUNDS_MESSAGE: &str = "This wallet has no confirmed funds left to pay the \
    higher fee. Wait for a pending payment to confirm, then try again.";

impl FeeBumpError {
    /// A message safe to show the user, where `Display` can carry internal error text for logs
    pub fn user_message(&self) -> String {
        match self {
            Self::NotFound => "This transaction is no longer in the wallet.".to_string(),
            Self::InitialScanIncomplete => {
                "Wait for the wallet to finish its first scan, then try again.".to_string()
            }
            Self::Blocked(blocker) => blocker.user_message(),
            Self::FeeRateTooLow { minimum } => format!(
                "The fee rate is too low to replace this transaction. Use at least {:.1} sat/vB.",
                sat_per_vb(*minimum)
            ),
            Self::InsufficientFunds { .. } => INSUFFICIENT_FUNDS_MESSAGE.to_string(),
            Self::Build(_) => {
                "Unable to build the faster transaction. Please try again.".to_string()
            }
        }
    }
}

impl From<WalletManagerError> for FeeBumpError {
    fn from(error: WalletManagerError) -> Self {
        match error {
            WalletManagerError::InitialScanIncomplete => Self::InitialScanIncomplete,
            error => Self::Build(error.to_string()),
        }
    }
}

impl From<BuildFeeBumpError> for FeeBumpError {
    fn from(error: BuildFeeBumpError) -> Self {
        match error {
            BuildFeeBumpError::TransactionNotFound(_) => Self::NotFound,
            BuildFeeBumpError::TransactionConfirmed(_) => FeeBumpBlocker::Confirmed.into(),
            BuildFeeBumpError::IrreplaceableTransaction(_) => FeeBumpBlocker::NotReplaceable.into(),
            BuildFeeBumpError::FeeRateUnavailable | BuildFeeBumpError::UnknownUtxo(_) => {
                FeeBumpBlocker::UnknownFee.into()
            }
            error => Self::Build(error.to_string()),
        }
    }
}

impl From<CreateTxError> for FeeBumpError {
    fn from(error: CreateTxError) -> Self {
        match error {
            CreateTxError::FeeRateTooLow { required } => {
                Self::FeeRateTooLow { minimum: round_up_to_step(required) }
            }
            CreateTxError::CoinSelection(insufficient) => Self::InsufficientFunds {
                needed: insufficient.needed,
                available: insufficient.available,
            },
            error => Self::Build(error.to_string()),
        }
    }
}

/// A pending transaction this wallet sent and can replace
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceableTransaction {
    pub txid: Txid,
    pub fee: Amount,
    /// Weight of the signed transaction as broadcast
    pub weight: Weight,
}

impl ReplaceableTransaction {
    /// The original's fee rate, rounded up
    pub fn fee_rate(&self) -> BdkFeeRate {
        fee_rate_covering(self.fee, self.weight)
    }

    /// The lowest rate worth trying: one incremental relay step above the original's rate
    ///
    /// BIP 125 rule 6 only needs a higher rate than the original, but BDK's `build_fee_bump`
    /// refuses any rate below this step. A replacement that grows, by adding an input to pay the
    /// fee, could relay a little under it, and getting there would mean building at an absolute
    /// fee and searching coin selection by hand, so the wallet keeps BDK's floor. Rule 4 can
    /// need more when the replacement is smaller than the original, which
    /// [`Self::minimum_fee`] checks once the replacement's weight is known
    pub fn starting_fee_rate(&self) -> BdkFeeRate {
        let sat_per_kwu =
            self.fee_rate().to_sat_per_kwu().saturating_add(INCREMENTAL_RELAY_SAT_PER_KWU);

        round_up_to_step(BdkFeeRate::from_sat_per_kwu(sat_per_kwu))
    }

    /// The least a replacement of `weight` must pay: the original's fee plus its own relay at
    /// the incremental rate (BIP 125 rules 3 and 4)
    pub fn minimum_fee(&self, replacement_weight: Weight) -> Amount {
        let relay = BdkFeeRate::from_sat_per_kwu(INCREMENTAL_RELAY_SAT_PER_KWU)
            .fee_wu(replacement_weight)
            .unwrap_or(Amount::MAX);

        self.fee.checked_add(relay).unwrap_or(Amount::MAX)
    }

    /// The rate at which a replacement of `replacement_weight` pays [`Self::minimum_fee`]
    pub fn minimum_fee_rate(&self, replacement_weight: Weight) -> BdkFeeRate {
        let rate = fee_rate_covering(self.minimum_fee(replacement_weight), replacement_weight);

        round_up_to_step(rate.max(self.starting_fee_rate()))
    }
}

/// The lowest rate at which `weight` pays at least `fee`
///
/// Rounds up, where `Amount / Weight` rounds down and divides by zero on an empty weight
fn fee_rate_covering(fee: Amount, weight: Weight) -> BdkFeeRate {
    let weight = weight.to_wu().max(1);

    BdkFeeRate::from_sat_per_kwu(fee.to_sat().saturating_mul(1_000).div_ceil(weight))
}

/// Rounds `rate` up to the next [`FEE_RATE_STEP_SAT_PER_KWU`]
fn round_up_to_step(rate: BdkFeeRate) -> BdkFeeRate {
    let sat_per_kwu = rate.to_sat_per_kwu().div_ceil(FEE_RATE_STEP_SAT_PER_KWU);

    BdkFeeRate::from_sat_per_kwu(sat_per_kwu.saturating_mul(FEE_RATE_STEP_SAT_PER_KWU))
}

#[allow(clippy::cast_precision_loss)] // only formats a rate for an error message
fn sat_per_vb(rate: BdkFeeRate) -> f64 {
    rate.to_sat_per_kwu() as f64 / 250.0
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash as _;

    use super::*;

    #[test]
    fn a_smaller_replacement_needs_a_higher_rate_than_one_step_above_the_original() {
        // 1,410 sats at 564 WU (141 vB) is 10 sat/vB
        let original = ReplaceableTransaction {
            txid: Txid::from_byte_array([1; 32]),
            fee: Amount::from_sat(1_410),
            weight: Weight::from_wu(564),
        };
        assert_eq!(original.starting_fee_rate(), BdkFeeRate::from_sat_per_vb_u32(11));

        // the same size needs 1,410 + 141 = 1,551 sats, which 11 sat/vB pays exactly
        assert_eq!(original.minimum_fee(Weight::from_wu(564)), Amount::from_sat(1_551));
        assert_eq!(
            original.minimum_fee_rate(Weight::from_wu(564)),
            BdkFeeRate::from_sat_per_vb_u32(11)
        );

        // dropping the change output to 110 vB needs 1,410 + 110 = 1,520 sats, 13.82 sat/vB,
        // offered as 13.9 sat/vB
        let rate = original.minimum_fee_rate(Weight::from_wu(440));
        assert_eq!(rate, BdkFeeRate::from_sat_per_kwu(3_475));
        assert!(rate.fee_wu(Weight::from_wu(440)).unwrap() >= Amount::from_sat(1_520));
    }
}
