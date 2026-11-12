//! Speeding up a pending transaction this wallet sent, by replacing it with one that pays a
//! higher fee (BIP 125 RBF)
//!
//! The frontend opens it from a transaction's details. It loads the rates a replacement can pay
//! as the same fast, medium and slow options the send flow shows, each with its total fee, and
//! on continue builds the replacement and pushes the send flow's confirm screen for a hot wallet
//! or its hardware export screen for one whose keys live elsewhere

use std::sync::Arc;

use cove_types::{
    TxId,
    amount::Amount,
    fees::{
        FeeRateOption, FeeRateOptionWithTotalFee, FeeRateOptions, FeeRateOptionsWithTotalFee,
        FeeSpeed,
    },
};
use parking_lot::Mutex;
use tracing::{debug, warn};

use crate::{
    app::AppAction,
    fee_client::FEE_CLIENT,
    manager::{deferred_dispatch::DeferredDispatch, wallet_manager::RustWalletManager},
    router::RouteFactory,
    transaction::{
        FeeRate,
        fee_bump::{FeeBumpBlocker, FeeBumpError, INSUFFICIENT_FUNDS_MESSAGE},
    },
    wallet::metadata::WalletType,
};

use super::reconcile_channel::ReconcileChannel;

type Message = FeeBumpManagerReconcileMessage;
type Action = FeeBumpManagerAction;
type Reconciler = dyn FeeBumpManagerReconciler;

#[uniffi::export(callback_interface)]
pub trait FeeBumpManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    /// Tells the frontend to reconcile the manager changes
    fn reconcile(&self, message: Message);
    fn reconcile_many(&self, messages: Vec<Message>);
}

crate::manager::reconcile_channel::impl_reconcile_sink!(
    dyn FeeBumpManagerReconciler,
    Message,
    many
);

#[derive(Debug, Clone, uniffi::Enum)]
pub enum FeeBumpManagerReconcileMessage {
    UpdateStatus(FeeBumpStatus),
    UpdateSelectedFeeRate(Arc<FeeRateOptionWithTotalFee>),

    /// Building the replacement failed after the user picked a rate, the message is for an alert
    ContinueFailed(String),
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum FeeBumpManagerAction {
    /// Select a preset, or a custom option from [`RustFeeBumpManager::custom_fee_option`],
    /// which then joins the options as their custom row
    SelectFeeRate(Arc<FeeRateOptionWithTotalFee>),
    Continue,

    /// The screen is showing again, such as after the user came back from the confirm screen,
    /// so continue can build a new replacement
    ScreenAppeared,
}

/// Whether the transaction can be sped up, and at which rates
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FeeBumpStatus {
    Loading,
    Ready(FeeBumpOptions),
    Unavailable(FeeBumpUnavailable),
}

/// The rates a replacement can pay
#[derive(Debug, Clone, uniffi::Record)]
pub struct FeeBumpOptions {
    /// What the transaction being replaced pays
    pub original_fee: Arc<Amount>,
    pub original_fee_rate: Arc<FeeRate>,

    /// The lowest rate a node accepts the replacement at, a custom rate can't go below it
    pub minimum_fee_rate: Arc<FeeRate>,

    /// Network rates raised to at least [`Self::minimum_fee_rate`], with the replacement's total
    /// fee at each the wallet can pay. When it can pay none of them, the custom row holds the
    /// minimum rate instead
    pub fee_options: Arc<FeeRateOptionsWithTotalFee>,
}

/// Why the transaction can't be sped up
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FeeBumpUnavailable {
    Blocked(FeeBumpBlocker),

    /// No confirmed coin is left to pay even the lowest replacement rate
    InsufficientFunds {
        needed: Arc<Amount>,
        available: Arc<Amount>,
    },

    /// The fee server can't be reached, and the confirm screens need its rates as a new send
    /// does
    FeeRatesUnavailable,

    /// Anything else, with a message safe to show the user
    Failed {
        message: String,
    },
}

impl From<FeeBumpError> for FeeBumpUnavailable {
    fn from(error: FeeBumpError) -> Self {
        match error {
            FeeBumpError::Blocked(blocker) => Self::Blocked(blocker),
            FeeBumpError::InsufficientFunds { needed, available } => Self::InsufficientFunds {
                needed: Arc::new(needed.into()),
                available: Arc::new(available.into()),
            },
            error => Self::Failed { message: error.user_message() },
        }
    }
}

impl FeeBumpUnavailable {
    pub fn user_message(&self) -> String {
        match self {
            Self::Blocked(blocker) => blocker.user_message(),
            Self::InsufficientFunds { .. } => INSUFFICIENT_FUNDS_MESSAGE.to_string(),
            Self::FeeRatesUnavailable => {
                "Unable to load the current network fees. Check your connection and try again."
                    .to_string()
            }
            Self::Failed { message } => message.clone(),
        }
    }
}

#[uniffi::export]
impl FeeBumpUnavailable {
    #[uniffi::method(name = "userMessage")]
    fn ffi_user_message(&self) -> String {
        self.user_message()
    }
}

/// Where continue is, so a second tap can't build or push a second replacement
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContinueState {
    Idle,
    Building,

    /// The confirm screen was pushed, and stays the only one until this screen shows again
    Pushed,
}

#[derive(Debug)]
struct State {
    status: FeeBumpStatus,
    selected: Option<Arc<FeeRateOptionWithTotalFee>>,
    continue_state: ContinueState,
}

#[derive(Debug, uniffi::Object)]
pub struct RustFeeBumpManager {
    txid: bitcoin::Txid,
    wallet_manager: Arc<RustWalletManager>,
    state: Mutex<State>,
    reconciler: ReconcileChannel<Message>,
}

#[uniffi::export]
impl RustWalletManager {
    /// A manager that speeds up the pending transaction `tx_id`, loading its options right away
    #[uniffi::method]
    pub fn new_fee_bump_manager(self: Arc<Self>, tx_id: Arc<TxId>) -> Arc<RustFeeBumpManager> {
        let manager = Arc::new(RustFeeBumpManager::new(self, tx_id.0));

        let loader = manager.clone();
        cove_tokio::task::spawn(async move { loader.load().await });

        manager
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl RustFeeBumpManager {
    #[uniffi::method]
    pub fn listen_for_updates(&self, reconciler: Box<Reconciler>) {
        self.reconciler.listen_sink_async(reconciler);
    }

    /// The status when the frontend first renders, later changes arrive as reconcile messages
    #[uniffi::method]
    pub fn status(&self) -> FeeBumpStatus {
        self.state.lock().status.clone()
    }

    #[uniffi::method]
    pub fn selected_fee_rate(&self) -> Option<Arc<FeeRateOptionWithTotalFee>> {
        self.state.lock().selected.clone()
    }

    #[uniffi::method]
    pub fn dispatch(self: Arc<Self>, action: Action) {
        match action {
            Action::SelectFeeRate(fee_rate) => self.select_fee_rate(fee_rate),
            Action::Continue => self.continue_with_selected_fee_rate(),
            Action::ScreenAppeared => self.screen_appeared(),
        }
    }

    /// A custom rate with the replacement's total fee at it
    ///
    /// Fails with [`FeeBumpError::FeeRateTooLow`] below the rate a node would accept
    #[uniffi::method]
    pub async fn custom_fee_option(
        &self,
        fee_rate: Arc<FeeRate>,
    ) -> Result<Arc<FeeRateOptionWithTotalFee>, FeeBumpError> {
        let fee_rate = *fee_rate;
        let Some(fee_options) = self.fee_options() else {
            return Err(FeeBumpError::Build("the fee options are still loading".to_string()));
        };

        let total_fee = self.wallet_manager.fee_bump_total_fee(self.txid, fee_rate).await?;
        let fee_speed = fee_options.calculate_custom_fee_speed(fee_rate.sat_per_vb());
        let option = FeeRateOptionWithTotalFee { fee_speed, fee_rate, total_fee: Some(total_fee) };

        Ok(Arc::new(option))
    }
}

impl RustFeeBumpManager {
    fn new(wallet_manager: Arc<RustWalletManager>, txid: bitcoin::Txid) -> Self {
        let state = State {
            status: FeeBumpStatus::Loading,
            selected: None,
            continue_state: ContinueState::Idle,
        };

        Self {
            txid,
            wallet_manager,
            state: Mutex::new(state),
            reconciler: ReconcileChannel::new(10),
        }
    }

    fn fee_options(&self) -> Option<Arc<FeeRateOptionsWithTotalFee>> {
        match &self.state.lock().status {
            FeeBumpStatus::Ready(options) => Some(options.fee_options.clone()),
            FeeBumpStatus::Loading | FeeBumpStatus::Unavailable(_) => None,
        }
    }

    async fn load(self: Arc<Self>) {
        let (status, selected) = match self.ready_options().await {
            Ok(options) => {
                let selected = default_selection(&options.fee_options);
                (FeeBumpStatus::Ready(options), Some(selected))
            }

            Err(unavailable) => (FeeBumpStatus::Unavailable(unavailable), None),
        };

        {
            let mut state = self.state.lock();
            state.status = status.clone();
            state.selected.clone_from(&selected);
        }

        let mut messages = vec![Message::UpdateStatus(status)];
        messages.extend(selected.map(Message::UpdateSelectedFeeRate));
        self.reconciler.send_async(messages).await;
    }

    async fn ready_options(&self) -> Result<FeeBumpOptions, FeeBumpUnavailable> {
        let target = self.wallet_manager.fee_bump_target(self.txid).await.inspect_err(|error| {
            warn!("unable to speed up {}: {error:?}", self.txid);
        })?;

        // the minimum is the cheapest a replacement can be, so only failing there blocks the
        // screen; a network rate the wallet can't pay just shows no total
        let minimum = FeeRate::from(target.minimum_fee_rate);
        let minimum_total = self.total_fee(minimum).await?;

        let Some(base) = network_fee_rate_options().await else {
            return Err(FeeBumpUnavailable::FeeRatesUnavailable);
        };

        let base = raised_to(base, minimum);
        let slow = self.with_total_fee_if_payable(base.slow).await;
        let medium = self.with_total_fee_if_payable(base.medium).await;
        let fast = self.with_total_fee_if_payable(base.fast).await;

        let mut fee_options = FeeRateOptionsWithTotalFee { fast, medium, slow, custom: None };

        // with no network rate payable, the minimum is the one rate offered
        if slow.total_fee.is_none() {
            let fee_speed = fee_options.calculate_custom_fee_speed(minimum.sat_per_vb());
            let at_minimum = FeeRateOptionWithTotalFee {
                fee_speed,
                fee_rate: minimum,
                total_fee: Some(minimum_total),
            };
            fee_options = fee_options.add_custom_fee_rate(Arc::new(at_minimum));
        }

        Ok(FeeBumpOptions {
            original_fee: Arc::new(target.original.fee.into()),
            original_fee_rate: Arc::new(target.original.fee_rate().into()),
            minimum_fee_rate: Arc::new(minimum),
            fee_options: Arc::new(fee_options),
        })
    }

    async fn total_fee(&self, fee_rate: FeeRate) -> Result<Amount, FeeBumpError> {
        self.wallet_manager.fee_bump_total_fee(self.txid, fee_rate).await
    }

    /// Leaves the total out when the wallet can't pay this rate, continuing at it then explains
    async fn with_total_fee_if_payable(&self, option: FeeRateOption) -> FeeRateOptionWithTotalFee {
        match self.total_fee(option.fee_rate).await {
            Ok(total_fee) => FeeRateOptionWithTotalFee::new(option, total_fee),
            Err(error) => {
                debug!("no total fee at {} sat/vB: {error}", option.sat_per_vb());
                FeeRateOptionWithTotalFee::without_total(option)
            }
        }
    }

    fn select_fee_rate(&self, fee_rate: Arc<FeeRateOptionWithTotalFee>) {
        let mut messages = Vec::new();

        {
            let mut state = self.state.lock();
            state.selected = Some(fee_rate.clone());

            // a custom rate becomes the options' custom row, so the picker can show it
            if let (FeeSpeed::Custom { .. }, FeeBumpStatus::Ready(options)) =
                (fee_rate.fee_speed, &mut state.status)
            {
                options.fee_options =
                    Arc::new(options.fee_options.add_custom_fee_rate(fee_rate.clone()));
                messages.push(Message::UpdateStatus(state.status.clone()));
            }
        }

        messages.push(Message::UpdateSelectedFeeRate(fee_rate));
        self.reconciler.send(messages);
    }

    fn screen_appeared(&self) {
        let mut state = self.state.lock();
        if state.continue_state == ContinueState::Pushed {
            state.continue_state = ContinueState::Idle;
        }
    }

    fn continue_with_selected_fee_rate(self: Arc<Self>) {
        let selected = {
            let mut state = self.state.lock();
            let Some(selected) = state.selected.clone() else { return };
            if state.continue_state != ContinueState::Idle {
                return;
            }

            state.continue_state = ContinueState::Building;
            selected
        };

        cove_tokio::task::spawn(async move {
            let result = self.push_confirm_route(selected.fee_rate).await;

            let continue_state =
                if result.is_ok() { ContinueState::Pushed } else { ContinueState::Idle };
            self.state.lock().continue_state = continue_state;

            let Err(error) = result else { return };
            warn!("unable to build the replacement for {}: {error:?}", self.txid);

            // a transaction that confirmed or was replaced meanwhile can't be sped up any more
            let mut messages = vec![Message::ContinueFailed(error.user_message())];
            if let FeeBumpError::Blocked(blocker) = error {
                let status = FeeBumpStatus::Unavailable(FeeBumpUnavailable::Blocked(blocker));
                self.state.lock().status = status.clone();
                messages.push(Message::UpdateStatus(status));
            }

            self.reconciler.send_async(messages).await;
        });
    }

    /// Builds the replacement and sends the frontend to sign it, the way a new send would
    async fn push_confirm_route(&self, fee_rate: FeeRate) -> Result<(), FeeBumpError> {
        let details = self.wallet_manager.confirm_fee_bump_txn(self.txid, fee_rate).await?;
        let details = Arc::new(details);
        let metadata = self.wallet_manager.current_metadata();

        let route = match metadata.wallet_type {
            WalletType::Hot => RouteFactory::new().send_confirm(metadata.id, details, None),

            // the export screen signs a saved draft, which the replacement must be too
            WalletType::Cold | WalletType::XpubOnly => {
                self.wallet_manager.save_unsigned_transaction(details.clone())?;
                RouteFactory::new().send_hardware_export(metadata.id, details)
            }

            WalletType::WatchOnly => return Err(FeeBumpBlocker::WatchOnly.into()),
        };

        let mut deferred = DeferredDispatch::<AppAction>::new();
        deferred.queue(AppAction::PushRoute(route));

        Ok(())
    }
}

/// Medium when the wallet can pay it, as a new send starts, else the cheapest rate it can pay
fn default_selection(fee_options: &FeeRateOptionsWithTotalFee) -> Arc<FeeRateOptionWithTotalFee> {
    [fee_options.medium, fee_options.slow]
        .into_iter()
        .chain(fee_options.custom)
        .find(|option| option.total_fee.is_some())
        .map_or_else(|| Arc::new(fee_options.medium), Arc::new)
}

/// The fee server's current rates, or none when it can't be reached
async fn network_fee_rate_options() -> Option<FeeRateOptions> {
    let fees = FEE_CLIENT
        .fetch_and_get_fees()
        .await
        .inspect_err(|error| warn!("unable to fetch fee rates for a fee bump: {error}"))
        .ok()?;

    fees.fee_rate_options()
        .inspect_err(|error| warn!("ignoring invalid fee rates for a fee bump: {error}"))
        .ok()
}

/// Raises every network rate to at least `minimum`
///
/// A replacement under the minimum would be refused, so a rate the network calls fast can still
/// be raised when the original already paid more
fn raised_to(base: FeeRateOptions, minimum: FeeRate) -> FeeRateOptions {
    let raise = |option: FeeRateOption| FeeRateOption {
        fee_speed: option.fee_speed,
        fee_rate: option.fee_rate.max(minimum),
    };

    FeeRateOptions { fast: raise(base.fast), medium: raise(base.medium), slow: raise(base.slow) }
}
