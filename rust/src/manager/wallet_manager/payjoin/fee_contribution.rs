//! What a Payjoin payment offers toward the fee of the coin the receiver adds
//!
//! A receiver that adds a coin makes the transaction larger, so keeping the payment's fee rate
//! costs more. BIP 78 lets the sender allow the receiver to take that extra fee from the
//! sender's change, up to a maximum the sender names. Cove fixes that maximum before the user
//! confirms, so the confirm screen can show the most the payment can cost

use std::{fmt, str::FromStr};

use bitcoin::{
    Amount as BdkAmount, FeeRate as BdkFeeRate, Script, Transaction, TxOut, Weight,
    transaction::InputWeightPrediction,
};
use cove_types::{amount::Amount, confirm::ConfirmDetails};

use crate::wallet::WalletAddressType;

/// How much of a Payjoin's extra fee this wallet offers to pay for the coin the receiver adds
///
/// Chosen once in Settings and applied to every Payjoin payment
#[derive(Debug, Clone, Copy, Default, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PayjoinFeeHelp {
    /// Up to the fee of one more input like this wallet's own, at the payment's fee rate, which
    /// is what most wallets offer
    #[default]
    UpToOneInput,

    /// Nothing; the payment pays exactly the fee the confirm screen shows, and a receiver that
    /// will not pay for its own coin declines, so the payment is sent normally
    DontHelp,
}

impl PayjoinFeeHelp {
    const UP_TO_ONE_INPUT: &'static str = "up_to_one_input";
    const DONT_HELP: &'static str = "dont_help";
}

// the persisted form in the global config table
impl fmt::Display for PayjoinFeeHelp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UpToOneInput => f.write_str(Self::UP_TO_ONE_INPUT),
            Self::DontHelp => f.write_str(Self::DONT_HELP),
        }
    }
}

impl FromStr for PayjoinFeeHelp {
    type Err = UnknownPayjoinFeeHelp;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            Self::UP_TO_ONE_INPUT => Ok(Self::UpToOneInput),
            Self::DONT_HELP => Ok(Self::DontHelp),
            _ => Err(UnknownPayjoinFeeHelp),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown Payjoin fee help setting")]
pub struct UnknownPayjoinFeeHelp;

/// The most a Payjoin receiver may take from this wallet's change to pay for the coin it adds
///
/// Exists only for a payment that offers something: fee help is on, the payment has a change
/// output, and that change can give more than nothing while staying spendable. It is fixed
/// before the user confirms, so the confirm screen's figures are exactly what the payment
/// offers, and rust-payjoin rejects a proposal that takes more
#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Object)]
pub struct PayjoinFeeContribution {
    max: BdkAmount,
    change_index: usize,
    max_fee_total: BdkAmount,
    max_spending_amount: BdkAmount,
}

impl PayjoinFeeContribution {
    /// What a Payjoin payment of `details` offers under `help`, `None` when it offers nothing
    ///
    /// `is_mine` reports whether an output script belongs to the sending wallet, which tells
    /// the change apart from the payment
    pub(crate) fn offer(
        help: PayjoinFeeHelp,
        details: &ConfirmDetails,
        address_type: WalletAddressType,
        is_mine: impl Fn(&Script) -> bool,
    ) -> Option<Self> {
        let payee = details.sending_to.script_pubkey();
        let (change_index, change) = change_output(&details.psbt.unsigned_tx, &payee, is_mine)?;
        let max = max_contribution(help, details.fee_rate.into(), address_type, change)?;

        Some(Self {
            max,
            change_index,
            max_fee_total: details.fee_total.0.checked_add(max)?,
            max_spending_amount: details.spending_amount.0.checked_add(max)?,
        })
    }

    pub(crate) const fn max_bdk(&self) -> BdkAmount {
        self.max
    }

    pub(crate) const fn change_index(&self) -> usize {
        self.change_index
    }
}

#[uniffi::export]
impl PayjoinFeeContribution {
    /// The most the receiver may take from the change, never zero
    pub fn max(&self) -> Amount {
        self.max.into()
    }

    /// The payment's network fee if the receiver takes all of `max`
    pub fn max_fee_total(&self) -> Amount {
        self.max_fee_total.into()
    }

    /// What the payment takes from the wallet, amount and fees, if the receiver takes all of
    /// `max`
    pub fn max_spending_amount(&self) -> Amount {
        self.max_spending_amount.into()
    }
}

/// The payment's one change output and its index, `None` when the payment has no change
///
/// Payjoin pays a single recipient, so the change is the one output that pays this wallet
/// and is not the recipient's
fn change_output<'a>(
    tx: &'a Transaction,
    payee: &Script,
    is_mine: impl Fn(&Script) -> bool,
) -> Option<(usize, &'a TxOut)> {
    let mut change = tx
        .output
        .iter()
        .enumerate()
        .filter(|(_, output)| output.script_pubkey.as_script() != payee)
        .filter(|(_, output)| is_mine(&output.script_pubkey));

    let found = change.next()?;
    change.next().is_none().then_some(found)
}

/// The most the receiver may take from `change`, `None` when that is nothing
///
/// One input of the sender's type at the payment's fee rate, the amount BIP 78 and most
/// wallets offer. The receiver lowers the change by what it takes without checking dust, so
/// the offer leaves the change at least at its dust limit
pub(crate) fn max_contribution(
    help: PayjoinFeeHelp,
    fee_rate: BdkFeeRate,
    address_type: WalletAddressType,
    change: &TxOut,
) -> Option<BdkAmount> {
    if help == PayjoinFeeHelp::DontHelp {
        return None;
    }

    let one_input = fee_rate.checked_mul_by_weight(sender_input_weight(address_type))?;
    let spare = change.value.checked_sub(change.script_pubkey.minimal_non_dust())?;
    let max = one_input.min(spare);

    (max > BdkAmount::ZERO).then_some(max)
}

/// The weight one more input of this wallet's type adds once signed
fn sender_input_weight(address_type: WalletAddressType) -> Weight {
    let prediction = match address_type {
        WalletAddressType::NativeSegwit => InputWeightPrediction::P2WPKH_MAX,

        // a script sig pushing the 22-byte witness program, then a P2WPKH witness
        WalletAddressType::WrappedSegwit => InputWeightPrediction::from_slice(23, &[72, 33]),

        WalletAddressType::Legacy => InputWeightPrediction::P2PKH_COMPRESSED_MAX,
    };

    // the outpoint and sequence every input carries outside its scripts
    prediction.weight() + Weight::from_non_witness_data_size(32 + 4 + 4)
}

#[cfg(test)]
mod tests {
    use bitcoin::{ScriptBuf, hashes::Hash as _};

    use super::*;

    fn rate(sat_per_vb: u64) -> BdkFeeRate {
        BdkFeeRate::from_sat_per_vb(sat_per_vb).expect("a valid fee rate")
    }

    fn p2wpkh_script(seed: u8) -> ScriptBuf {
        let hash = bitcoin::WPubkeyHash::from_byte_array([seed; 20]);
        ScriptBuf::new_p2wpkh(&hash)
    }

    fn change(sats: u64) -> TxOut {
        TxOut { value: BdkAmount::from_sat(sats), script_pubkey: p2wpkh_script(1) }
    }

    fn sats(amount: Option<BdkAmount>) -> Option<u64> {
        amount.map(BdkAmount::to_sat)
    }

    /// One input's fee at 10 sat/vB for each address type a Cove wallet can have
    #[test]
    fn one_input_at_the_payment_fee_rate_by_script_type() {
        let help = PayjoinFeeHelp::UpToOneInput;
        let change = change(100_000);

        let native = max_contribution(help, rate(10), WalletAddressType::NativeSegwit, &change);
        let wrapped = max_contribution(help, rate(10), WalletAddressType::WrappedSegwit, &change);
        let legacy = max_contribution(help, rate(10), WalletAddressType::Legacy, &change);

        // 68, 91 and 148 vB inputs
        assert_eq!(sats(native), Some(680));
        assert_eq!(sats(wrapped), Some(910));
        assert_eq!(sats(legacy), Some(1_480));
    }

    /// A fractional fee rate rounds the offer up to the next sat, as rust-payjoin's own
    /// recommendation does
    #[test]
    fn a_fractional_fee_rate_rounds_up() {
        let fee_rate = BdkFeeRate::from_sat_per_kwu(376);
        let max = max_contribution(
            PayjoinFeeHelp::UpToOneInput,
            fee_rate,
            WalletAddressType::NativeSegwit,
            &change(100_000),
        );

        // 376 sat/kwu × 272 wu = 102.272 sats
        assert_eq!(sats(max), Some(103));
    }

    #[test]
    fn fee_help_off_offers_nothing() {
        let max = max_contribution(
            PayjoinFeeHelp::DontHelp,
            rate(10),
            WalletAddressType::NativeSegwit,
            &change(100_000),
        );

        assert_eq!(max, None);
    }

    /// A small change gives only what keeps it above its 294 sat P2WPKH dust limit, and a
    /// change at the limit gives nothing
    #[test]
    fn a_small_change_stays_spendable() {
        let help = PayjoinFeeHelp::UpToOneInput;
        let native = WalletAddressType::NativeSegwit;

        assert_eq!(sats(max_contribution(help, rate(10), native, &change(500))), Some(206));
        assert_eq!(max_contribution(help, rate(10), native, &change(294)), None);
        assert_eq!(max_contribution(help, rate(10), native, &change(100)), None);
    }

    fn payment(outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: Vec::new(),
            output: outputs,
        }
    }

    fn payee() -> TxOut {
        TxOut { value: BdkAmount::from_sat(40_000), script_pubkey: p2wpkh_script(2) }
    }

    fn mine(script: &Script) -> bool {
        script == p2wpkh_script(1).as_script()
    }

    #[test]
    fn the_change_is_the_wallets_output_that_is_not_the_payment() {
        let tx = payment(vec![payee(), change(59_000)]);
        let payee_script = payee().script_pubkey;

        let found = change_output(&tx, &payee_script, mine);

        assert_eq!(found.map(|(index, output)| (index, output.value.to_sat())), Some((1, 59_000)));
    }

    /// A send-max pays only the recipient, so there is no change to take a contribution from
    #[test]
    fn a_payment_without_change_offers_nothing() {
        let tx = payment(vec![payee()]);

        assert_eq!(change_output(&tx, &payee().script_pubkey, mine), None);
    }

    /// The change output, and so the contribution, must be unambiguous
    #[test]
    fn two_wallet_outputs_are_not_a_change() {
        let tx = payment(vec![payee(), change(30_000), change(29_000)]);

        assert_eq!(change_output(&tx, &payee().script_pubkey, mine), None);
    }

    #[test]
    fn the_setting_round_trips_through_its_stored_form() {
        for help in [PayjoinFeeHelp::UpToOneInput, PayjoinFeeHelp::DontHelp] {
            assert_eq!(help.to_string().parse::<PayjoinFeeHelp>(), Ok(help));
        }

        assert_eq!("up_to_one_input".parse(), Ok(PayjoinFeeHelp::UpToOneInput));
        assert_eq!("dont_help".parse(), Ok(PayjoinFeeHelp::DontHelp));
        assert!("".parse::<PayjoinFeeHelp>().is_err());
    }

    #[test]
    fn the_default_offers_one_input() {
        assert_eq!(PayjoinFeeHelp::default(), PayjoinFeeHelp::UpToOneInput);
    }
}
