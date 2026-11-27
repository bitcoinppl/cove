//! Replacing a pending transaction with one that pays a higher fee (BIP 125 RBF)
//!
//! The wallet actor decides whether a transaction can be fee bumped and builds the replacement.
//! This module holds the rules both depend on, so the eligibility checks, the fee floor and the
//! errors shown to the user stay in one place

use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
};

use bdk_wallet::{
    Wallet as BdkWallet, WalletTx,
    chain::ChainPosition,
    error::{BuildFeeBumpError, CreateTxError},
};
use bitcoin::{
    Amount, FeeRate as BdkFeeRate, OutPoint, ScriptBuf, Transaction as BdkTransaction, Txid, Weight,
};
use tracing::debug;

use crate::{
    manager::wallet_manager::WalletManagerError,
    transaction::{Amount as CoveAmount, FeeRate, TxId},
    wallet::metadata::{WalletMetadata, WalletType},
};

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

    /// Carries the underlying error for logs through `Debug`, while `Display`, which crosses
    /// the FFI, stays generic
    #[error("unable to build the replacement")]
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

/// Where a replacement takes its higher fee from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementFunding {
    /// The change output, with a confirmed coin added when the change can't cover it
    Change,

    /// The one output the transaction has, which pays `sent`
    ///
    /// A send with no change, such as a Max send, has nothing else to take the fee from. The
    /// replacement keeps its inputs and adds no coin, so the recipient receives less by the
    /// extra fee, where adding a coin would spend money the user chose not to send
    Amount { sent: Amount },
}

impl ReplacementFunding {
    /// How `tx` funds its replacement: from its one output when it has no other, else from
    /// its change
    pub fn of(tx: &BdkTransaction) -> Self {
        match tx.output.as_slice() {
            [only] => Self::Amount { sent: only.value },
            _ => Self::Change,
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
    pub funding: ReplacementFunding,
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

/// What a wallet needs before it can replace any of its pending transactions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBumpWallet {
    pub wallet_type: WalletType,

    /// Until the first full scan, the wallet may not know every coin a replacement could spend
    pub completed_initial_scan: bool,
}

impl From<&WalletMetadata> for FeeBumpWallet {
    fn from(metadata: &WalletMetadata) -> Self {
        Self {
            wallet_type: metadata.wallet_type,
            completed_initial_scan: metadata.internal.performed_full_scan_at.is_some(),
        }
    }
}

/// The pending transaction `txid` if `wallet` can replace it
///
/// Looks `txid` up in the canonical view, so it reports a transaction that lost to a double
/// spend as [`FeeBumpBlocker::Replaced`]
pub fn replaceable_transaction(
    bdk: &BdkWallet,
    wallet: FeeBumpWallet,
    txid: Txid,
) -> Result<ReplaceableTransaction, FeeBumpError> {
    if !wallet.completed_initial_scan {
        return Err(FeeBumpError::InitialScanIncomplete);
    }

    // a transaction the graph holds but the canonical view drops lost to a double spend
    let Some(canonical) = bdk.get_tx(txid) else {
        return match bdk.tx_graph().get_tx(txid) {
            Some(_) => Err(FeeBumpBlocker::Replaced.into()),
            None => Err(FeeBumpError::NotFound),
        };
    };

    replaceable_canonical(bdk, wallet, &canonical, |txid| bdk.get_tx(txid).is_some())
}

/// Whether `wallet` can replace `canonical`, a transaction in its canonical view
///
/// The one place the eligibility rules live, so the transaction list's Bump Fee and the fee
/// bump screen can't disagree. It checks only what the wallet already knows, without building
/// a replacement, so it is cheap enough to run for every pending row. `is_canonical` says
/// whether another transaction is in the same canonical view, which a caller holding the whole
/// view answers without canonicalizing the graph again
pub fn check_replaceable(
    bdk: &BdkWallet,
    wallet: FeeBumpWallet,
    canonical: &WalletTx<'_>,
    is_canonical: impl Fn(Txid) -> bool,
) -> Result<ReplaceableTransaction, FeeBumpError> {
    if !wallet.completed_initial_scan {
        return Err(FeeBumpError::InitialScanIncomplete);
    }

    replaceable_canonical(bdk, wallet, canonical, is_canonical)
}

fn replaceable_canonical(
    bdk: &BdkWallet,
    wallet: FeeBumpWallet,
    canonical: &WalletTx<'_>,
    is_canonical: impl Fn(Txid) -> bool,
) -> Result<ReplaceableTransaction, FeeBumpError> {
    if canonical.chain_position.is_confirmed() {
        return Err(FeeBumpBlocker::Confirmed.into());
    }

    let txid = canonical.tx_node.txid;
    let tx = canonical.tx_node.tx.as_ref();
    let graph = bdk.tx_graph();
    let input_is_ours = |input: &bitcoin::TxIn| {
        graph
            .get_txout(input.previous_output)
            .is_some_and(|prevout| bdk.is_mine(prevout.script_pubkey.clone()))
    };

    if !tx.input.iter().any(input_is_ours) {
        return Err(FeeBumpBlocker::NotSentByWallet.into());
    }

    if wallet.wallet_type == WalletType::WatchOnly {
        return Err(FeeBumpBlocker::WatchOnly.into());
    }

    if !tx.input.iter().all(input_is_ours) {
        return Err(FeeBumpBlocker::ForeignInputs.into());
    }

    if !tx.is_explicitly_rbf() {
        return Err(FeeBumpBlocker::NotReplaceable.into());
    }

    let external_outputs =
        tx.output.iter().filter(|output| !bdk.is_mine(output.script_pubkey.clone())).count();

    if external_outputs > 1 {
        return Err(FeeBumpBlocker::MultipleRecipients.into());
    }

    if let Some(spender) = pending_spender_of_outputs(bdk, txid, tx, is_canonical) {
        debug!("{txid} can't be fee bumped, pending {spender} spends from it");
        return Err(FeeBumpBlocker::PendingDescendant.into());
    }

    let fee = bdk.calculate_fee(tx).map_err(|_| FeeBumpBlocker::UnknownFee)?;

    Ok(ReplaceableTransaction {
        txid,
        fee,
        weight: tx.weight(),
        funding: ReplacementFunding::of(tx),
    })
}

/// A pending transaction in the wallet that spends one of `tx`'s outputs
///
/// Replacing `tx` would drop it too, and BIP 125 would make the replacement pay its fee,
/// so the descendant is the one to speed up instead
fn pending_spender_of_outputs(
    bdk: &BdkWallet,
    txid: Txid,
    tx: &BdkTransaction,
    is_canonical: impl Fn(Txid) -> bool,
) -> Option<Txid> {
    let graph = bdk.tx_graph();

    (0..tx.output.len()).find_map(|vout| {
        let outpoint = OutPoint { txid, vout: u32::try_from(vout).ok()? };

        graph.outspends(outpoint).iter().copied().find(|spender| is_canonical(*spender))
    })
}

/// The scripts `tx` pays outside this wallet, which identify the payment it makes
///
/// A fee bump keeps them, so two transactions that spend the same coins and pay the same
/// outside scripts are versions of one payment, where a double spend from another device can
/// pay someone else entirely
pub fn outside_scripts(bdk: &BdkWallet, tx: &BdkTransaction) -> BTreeSet<ScriptBuf> {
    tx.output
        .iter()
        .map(|output| output.script_pubkey.clone())
        .filter(|script| !bdk.is_mine(script.clone()))
        .collect()
}

// MARK: fee history

/// A fee an earlier version of a payment paid before a fee bump replaced it
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct PreviousFee {
    pub tx_id: Arc<TxId>,
    pub fee: Arc<CoveAmount>,
    pub fee_rate: Arc<FeeRate>,
}

/// Shown on the confirm screen when the transaction being signed replaces a pending one
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct FeeBumpConfirmContext {
    /// The pending transaction the signed one takes the place of
    pub replaces: Arc<TxId>,
    pub previous_fee: Arc<CoveAmount>,
    pub previous_fee_rate: Arc<FeeRate>,

    /// How much less the recipient receives than the pending transaction paid them, when the
    /// higher fee comes out of the amount sent
    pub recipient_receives_less: Option<Arc<CoveAmount>>,
}

/// An earlier version of the payment a transaction makes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EarlierVersion {
    txid: Txid,
    fee: Amount,
    fee_rate: BdkFeeRate,
    first_seen: u64,

    /// Whether the wallet still lists it, as it does the pending one a new replacement replaces
    canonical: bool,
}

impl EarlierVersion {
    /// Oldest first: by when the wallet first saw it, then by fee, since each bump pays more
    fn sort_key(&self) -> (u64, Amount) {
        (self.first_seen, self.fee)
    }
}

impl From<EarlierVersion> for PreviousFee {
    fn from(version: EarlierVersion) -> Self {
        Self {
            tx_id: Arc::new(version.txid.into()),
            fee: Arc::new(version.fee.into()),
            fee_rate: Arc::new(version.fee_rate.into()),
        }
    }
}

/// Every fee the payment `tx` makes paid before, oldest first
///
/// Derived from the replaced transactions the wallet's graph keeps, so it survives restarts
/// with no table of its own: the unconfirmed transactions that spend a coin `tx` spends, pay
/// the same outside scripts, and pay less than `fee`, as BIP 125 makes every replacement pay
/// more than what it replaces. A wallet restored elsewhere never saw the earlier versions, so
/// its history starts at `tx`
pub fn fee_history(bdk: &BdkWallet, tx: &BdkTransaction, fee: Amount) -> Vec<PreviousFee> {
    let mut versions = earlier_versions(bdk, tx, fee);
    versions.sort_by_key(EarlierVersion::sort_key);
    versions.into_iter().map(PreviousFee::from).collect()
}

/// The pending payment `tx` would replace if broadcast, when `tx` is a fee bump of it
pub fn replaced_payment(
    bdk: &BdkWallet,
    tx: &BdkTransaction,
    fee: Amount,
) -> Option<FeeBumpConfirmContext> {
    let version = earlier_versions(bdk, tx, fee)
        .into_iter()
        .filter(|version| version.canonical)
        .max_by_key(EarlierVersion::sort_key)?;

    let replaced = bdk.tx_graph().get_tx(version.txid)?;
    let previous = PreviousFee::from(version);

    Some(FeeBumpConfirmContext {
        replaces: previous.tx_id,
        previous_fee: previous.fee,
        previous_fee_rate: previous.fee_rate,
        recipient_receives_less: amount_taken_from_recipient(&replaced, tx)
            .map(|less| Arc::new(less.into())),
    })
}

/// What `replacement` takes from the one output of `replaced` to pay its higher fee, when both
/// pay a single output to the same script, as a bump of a send with no change does
fn amount_taken_from_recipient(
    replaced: &BdkTransaction,
    replacement: &BdkTransaction,
) -> Option<Amount> {
    let ([before], [after]) = (replaced.output.as_slice(), replacement.output.as_slice()) else {
        return None;
    };

    if before.script_pubkey != after.script_pubkey {
        return None;
    }

    before.value.checked_sub(after.value).filter(|less| *less > Amount::ZERO)
}

fn earlier_versions(bdk: &BdkWallet, tx: &BdkTransaction, fee: Amount) -> Vec<EarlierVersion> {
    let graph = bdk.tx_graph();
    let recipients = outside_scripts(bdk, tx);

    // a version that spent several of the same coins conflicts once per coin
    let conflicts = graph.direct_conflicts(tx).map(|(_, txid)| txid).collect::<HashSet<_>>();

    conflicts
        .into_iter()
        .filter_map(|txid| {
            let node = graph.get_tx_node(txid)?;
            if outside_scripts(bdk, &node.tx) != recipients {
                return None;
            }

            // a confirmed conflict is what replaced `tx`, never an earlier version of it
            let position = bdk.get_tx(txid).map(|canonical| canonical.chain_position);
            if position.as_ref().is_some_and(ChainPosition::is_confirmed) {
                return None;
            }

            let version_fee = bdk.calculate_fee(&node.tx).ok().filter(|paid| *paid < fee)?;
            let fee_rate = bdk.calculate_fee_rate(&node.tx).ok()?;
            let first_seen = node.first_seen.or(node.last_seen).unwrap_or_default();
            let canonical = position.is_some();

            Some(EarlierVersion { txid, fee: version_fee, fee_rate, first_seen, canonical })
        })
        .collect()
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
            funding: ReplacementFunding::Change,
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
