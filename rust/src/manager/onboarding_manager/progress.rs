use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    database::{Database, global_config::GlobalConfigKey},
    network::Network,
    wallet::metadata::{WalletId, WalletMode},
    word_validator::WordValidator,
};

use super::{CreatedWalletFlow, FlowState, OnboardingBranch, RustOnboardingManager, TermsContext};

#[derive(Debug, Clone)]
pub(crate) struct InitialFlowResolution {
    pub(crate) flow: FlowState,
    pub(crate) clear_persisted_progress: bool,
    pub(crate) start_cloud_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OnboardingProgress {
    CreatedWallet {
        wallet_id: WalletId,
        branch: OnboardingBranch,
        network: Network,
        wallet_mode: WalletMode,
        secret_words_saved: bool,
        cloud_backup_enabled: bool,
    },
}

impl RustOnboardingManager {
    pub(crate) fn load_onboarding_progress() -> Option<OnboardingProgress> {
        let config = &Database::global().global_config;
        let raw = match config.get(GlobalConfigKey::OnboardingProgress) {
            Ok(Some(raw)) => raw,
            Ok(None) => return None,
            Err(error) => {
                warn!("Onboarding: failed to load persisted onboarding progress: {error}");
                return None;
            }
        };

        match serde_json::from_str(&raw) {
            Ok(progress) => Some(progress),
            Err(error) => {
                warn!("Onboarding: invalid persisted onboarding progress: {error}");
                if let Err(delete_error) = config.delete(GlobalConfigKey::OnboardingProgress) {
                    warn!(
                        "Onboarding: failed to clear invalid onboarding progress: {delete_error}"
                    );
                }
                None
            }
        }
    }

    pub(crate) fn sync_onboarding_progress(progress: Option<OnboardingProgress>) {
        let config = &Database::global().global_config;
        let current = config.get(GlobalConfigKey::OnboardingProgress).ok().flatten();

        match progress {
            Some(progress) => match serde_json::to_string(&progress) {
                Ok(serialized) => {
                    if current.as_deref() == Some(serialized.as_str()) {
                        return;
                    }
                    if let Err(error) = config.set(GlobalConfigKey::OnboardingProgress, serialized)
                    {
                        warn!("Onboarding: failed to persist onboarding progress: {error}");
                    }
                }
                Err(error) => warn!("Onboarding: failed to encode onboarding progress: {error}"),
            },
            None => {
                if current.is_none() {
                    return;
                }
                if let Err(error) = config.delete(GlobalConfigKey::OnboardingProgress) {
                    warn!("Onboarding: failed to clear onboarding progress: {error}");
                }
            }
        }
    }
}

impl From<CreatedWalletFlow> for OnboardingProgress {
    fn from(flow: CreatedWalletFlow) -> Self {
        Self::CreatedWallet {
            wallet_id: flow.wallet_id,
            branch: flow.branch,
            network: flow.network,
            wallet_mode: flow.wallet_mode,
            secret_words_saved: flow.secret_words_saved,
            cloud_backup_enabled: flow.cloud_backup_enabled,
        }
    }
}

impl OnboardingProgress {
    pub(crate) fn restore_flow<F>(&self, load_mnemonic: F) -> Option<FlowState>
    where
        F: FnOnce(&WalletId, Network, WalletMode) -> Option<bip39::Mnemonic>,
    {
        match self {
            Self::CreatedWallet {
                wallet_id,
                branch,
                network,
                wallet_mode,
                secret_words_saved,
                cloud_backup_enabled,
            } => {
                let mnemonic = load_mnemonic(wallet_id, *network, *wallet_mode)?;
                let created_words = mnemonic.words().map(str::to_string).collect();

                Some(FlowState::BackupWallet(CreatedWalletFlow {
                    branch: *branch,
                    wallet_id: wallet_id.clone(),
                    network: *network,
                    wallet_mode: *wallet_mode,
                    created_words,
                    word_validator: Arc::new(WordValidator::new(mnemonic)),
                    cloud_backup_enabled: *cloud_backup_enabled,
                    secret_words_saved: *secret_words_saved,
                }))
            }
        }
    }
}

fn default_initial_flow(has_wallets: bool) -> FlowState {
    if has_wallets {
        FlowState::terms(TermsContext::SelectLatestOrNew, None)
    } else {
        FlowState::Welcome { error: None }
    }
}

pub(crate) fn resolve_initial_flow<F>(
    progress: Option<OnboardingProgress>,
    has_wallets: bool,
    load_mnemonic: F,
) -> InitialFlowResolution
where
    F: FnOnce(&WalletId, Network, WalletMode) -> Option<bip39::Mnemonic>,
{
    match progress {
        Some(progress) => match progress.restore_flow(load_mnemonic) {
            Some(flow) => InitialFlowResolution {
                flow,
                clear_persisted_progress: false,
                start_cloud_check: false,
            },
            None => InitialFlowResolution {
                flow: default_initial_flow(has_wallets),
                clear_persisted_progress: true,
                start_cloud_check: !has_wallets,
            },
        },
        None => InitialFlowResolution {
            flow: default_initial_flow(has_wallets),
            clear_persisted_progress: false,
            start_cloud_check: !has_wallets,
        },
    }
}
