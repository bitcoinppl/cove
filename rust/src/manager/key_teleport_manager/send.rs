use std::{str::FromStr as _, sync::Arc};

use crate::database::global_config::SelectedWalletTarget;
use bdk_wallet::bitcoin::NetworkKind;
use cove_device::keychain::{Keychain, WalletSecret, WalletXprv};
use keyteleport::{NumericCode, Payload, SenderSession};
use zeroize::Zeroizing;

use crate::{
    database::Database,
    key_teleport::{KeyTeleportReceiverPacket, KeyTeleportSenderPacket},
    wallet::metadata::{WalletId, WalletMetadata, WalletType},
    wallet_identity::PublicWalletIdentity,
    wallet_secret::WalletSecretExt as _,
};

use super::{
    KeyTeleportAlert, KeyTeleportPassword, KeyTeleportSendReady, RustKeyTeleportManager,
    model::{Phase, SendPhase, StateMachine},
};
use cove_util::result_ext::ResultExt as _;

pub(crate) struct SendWorkflow<'a>(&'a RustKeyTeleportManager);

impl SendWorkflow<'_> {
    pub(crate) fn new(manager: &RustKeyTeleportManager) -> SendWorkflow<'_> {
        SendWorkflow(manager)
    }

    fn state_machine(&self) -> StateMachine<'_> {
        StateMachine::new(self.0)
    }

    pub(crate) fn start_from_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let (generation, packet) = {
            let model = self.0.model.lock();
            if model.phase.is_receive_direction() {
                return Err(KeyTeleportAlert::ConflictingTransferDirection);
            }

            let packet = match &model.phase {
                Phase::Send(SendPhase::ChooseWallet { packet, .. }) => Some(packet.clone()),
                _ => None,
            };

            (model.generation, packet)
        };

        let wallet = eligible_wallet_by_id(&wallet_id)?;

        let phase = match packet {
            Some(packet) => SendPhase::EnterCode { packet, wallet },
            None => SendPhase::AwaitReceiver { wallet },
        };

        if !self.state_machine().set_phase_if_current(generation, Phase::Send(phase)) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    pub(crate) fn start_with_receiver_packet(
        &self,
        packet: Arc<KeyTeleportReceiverPacket>,
    ) -> Result<(), KeyTeleportAlert> {
        let (generation, selected_wallet) = {
            let model = self.0.model.lock();
            let selected_wallet = match &model.phase {
                Phase::Send(SendPhase::AwaitReceiver { wallet }) => Some(wallet.clone()),
                _ => None,
            };

            (model.generation, selected_wallet)
        };

        if let Some(wallet) = selected_wallet {
            let phase = Phase::Send(SendPhase::EnterCode { packet, wallet });
            if !self.state_machine().set_phase_if_current(generation, phase) {
                return Err(KeyTeleportAlert::NoPendingSend);
            }

            return Ok(());
        }

        let eligible_wallets = eligible_wallets()?;
        if eligible_wallets.is_empty() {
            return Err(KeyTeleportAlert::NoEligibleWallets);
        }

        let phase = Phase::Send(SendPhase::ChooseWallet { packet, eligible_wallets });
        if !self.state_machine().set_phase_if_current(generation, phase) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    pub(crate) fn select_wallet(&self, wallet_id: WalletId) -> Result<(), KeyTeleportAlert> {
        let wallet = eligible_wallet_by_id(&wallet_id)?;
        let (generation, packet) = {
            let model = self.0.model.lock();
            let Phase::Send(SendPhase::ChooseWallet { packet, .. }) = &model.phase else {
                return Err(KeyTeleportAlert::NoPendingSend);
            };

            (model.generation, packet.clone())
        };

        let phase = Phase::Send(SendPhase::EnterCode { packet, wallet });
        if !self.state_machine().set_phase_if_current(generation, phase) {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }

    pub(crate) fn enter_receiver_code(&self, code: &str) -> Result<(), KeyTeleportAlert> {
        let code = NumericCode::from_str(code).map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        let (generation, packet, wallet) = {
            let model = self.0.model.lock();
            let Phase::Send(SendPhase::EnterCode { packet, wallet }) = &model.phase else {
                return Err(KeyTeleportAlert::NoPendingSend);
            };

            (model.generation, packet.clone(), wallet.clone())
        };

        let sender = SenderSession::new(packet.as_ref(), &code)
            .map_err(|_| KeyTeleportAlert::WrongReceiverCode)?;
        let wallet = current_send_wallet(&wallet.id)?;
        let keychain = Keychain::global();
        let secret =
            keychain.get_wallet_secret(&wallet.id)?.ok_or(KeyTeleportAlert::IneligibleWallet)?;

        if !is_supported_send_secret(&secret) {
            return Err(KeyTeleportAlert::IneligibleWallet);
        }

        let incoming_descriptors =
            secret.clone().into_descriptors(wallet.network, wallet.address_type);
        let incoming_identity = PublicWalletIdentity::from_descriptors(&incoming_descriptors);
        let existing_identity = PublicWalletIdentity::from_existing_wallet(&wallet, keychain)?
            .ok_or(KeyTeleportAlert::IneligibleWallet)?;

        if incoming_identity != existing_identity {
            return Err(KeyTeleportAlert::IneligibleWallet);
        }

        let payload = match secret {
            WalletSecret::Mnemonic(mnemonic) => Payload::mnemonic(mnemonic),
            WalletSecret::Xpriv(xpriv) => Payload::xprv(protocol_xprv(&xpriv)),
        }
        .map_err(|_| KeyTeleportAlert::InvalidPayload)?;

        let response = sender.send(payload).map_err_str(KeyTeleportAlert::Protocol)?;
        let state = KeyTeleportSendReady {
            selected_wallet: wallet,
            packet: Arc::new(KeyTeleportSenderPacket::from(response.packet)),
            password: Arc::new(KeyTeleportPassword::new(response.password)),
        };

        if !self
            .state_machine()
            .set_phase_if_current(generation, Phase::Send(SendPhase::Ready(state)))
        {
            return Err(KeyTeleportAlert::NoPendingSend);
        }

        Ok(())
    }
}

pub(crate) fn is_send_eligible_wallet_id(wallet_id: &WalletId) -> bool {
    match eligible_wallet_by_id(wallet_id) {
        Ok(_) => true,
        Err(KeyTeleportAlert::IneligibleWallet) => false,
        Err(error) => {
            tracing::warn!("unable to determine KeyTeleport send eligibility: {error}");
            false
        }
    }
}

fn eligible_wallets() -> Result<Vec<WalletMetadata>, KeyTeleportAlert> {
    let database = Database::global();
    let SelectedWalletTarget { network, mode } = database.global_config.wallet_target();
    let wallets = database.wallets.get_all(network, mode)?;
    let mut eligible = Vec::new();

    for wallet in wallets {
        match is_send_eligible(&wallet) {
            Ok(true) => eligible.push(wallet),
            Ok(false) => {}
            Err(error) => {
                let wallet_id = &wallet.id;
                tracing::warn!(
                    "unable to determine KeyTeleport send eligibility wallet_id={wallet_id}: {error}"
                );
            }
        }
    }

    Ok(eligible)
}

fn eligible_wallet_by_id(wallet_id: &WalletId) -> Result<WalletMetadata, KeyTeleportAlert> {
    let database = Database::global();
    let SelectedWalletTarget { network, mode } = database.global_config.wallet_target();
    let wallet = database
        .wallets
        .get(wallet_id, network, mode)?
        .ok_or(KeyTeleportAlert::IneligibleWallet)?;

    if !is_send_eligible(&wallet)? {
        return Err(KeyTeleportAlert::IneligibleWallet);
    }

    Ok(wallet)
}

fn current_send_wallet(wallet_id: &WalletId) -> Result<WalletMetadata, KeyTeleportAlert> {
    let database = Database::global();
    let SelectedWalletTarget { network, mode } = database.global_config.wallet_target();
    let wallet = database
        .wallets
        .get(wallet_id, network, mode)?
        .ok_or(KeyTeleportAlert::IneligibleWallet)?;

    if wallet.wallet_type != WalletType::Hot {
        return Err(KeyTeleportAlert::IneligibleWallet);
    }

    Ok(wallet)
}

fn is_send_eligible(wallet: &WalletMetadata) -> Result<bool, KeyTeleportAlert> {
    if wallet.wallet_type != WalletType::Hot {
        return Ok(false);
    }

    let Some(secret) = Keychain::global().get_wallet_secret(&wallet.id)? else {
        return Ok(false);
    };

    Ok(is_supported_send_secret(&secret))
}

fn is_supported_send_secret(secret: &WalletSecret) -> bool {
    match secret {
        WalletSecret::Mnemonic(mnemonic) => matches!(mnemonic.word_count(), 12 | 18 | 24),
        WalletSecret::Xpriv(_) => true,
    }
}

/// Encodes a stored master key the way the KeyTeleport payload accepts it
///
/// The wire format carries only the chain code and private key, and the keyteleport crate
/// accepts only mainnet-encoded keys. Stored keys can be tprv-encoded when a backup supplied
/// them that way, and the wallet's network comes from its metadata, not this version prefix
fn protocol_xprv(xprv: &WalletXprv) -> Zeroizing<String> {
    let mut xpriv = xprv.to_xpriv();
    xpriv.network = NetworkKind::Main;

    Zeroizing::new(xpriv.to_string())
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::wallet::metadata::{WalletId, WalletMetadata};

    use super::KeyTeleportAlert;

    pub(crate) fn eligible_wallets() -> Result<Vec<WalletMetadata>, KeyTeleportAlert> {
        super::eligible_wallets()
    }

    pub(crate) fn eligible_wallet_by_id(
        wallet_id: &WalletId,
    ) -> Result<WalletMetadata, KeyTeleportAlert> {
        super::eligible_wallet_by_id(wallet_id)
    }

    pub(crate) fn is_send_eligible(wallet: &WalletMetadata) -> Result<bool, KeyTeleportAlert> {
        super::is_send_eligible(wallet)
    }
}
