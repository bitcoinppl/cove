use bdk_wallet::bitcoin::{Network as BitcoinNetwork, bip32::Xpub, key::Secp256k1};
use cove_device::keychain::WalletSecret;

use crate::{
    keys::{Descriptor, DescriptorSecretKey, Descriptors},
    mnemonic::MnemonicExt as _,
    network::Network,
    wallet::WalletAddressType,
};

pub(crate) trait WalletSecretExt {
    fn into_descriptors(self, network: Network, address_type: WalletAddressType) -> Descriptors;

    fn xpub(&self, network: Network) -> Xpub;
}

impl WalletSecretExt for WalletSecret {
    fn into_descriptors(self, network: Network, address_type: WalletAddressType) -> Descriptors {
        match self {
            Self::Mnemonic(mnemonic) => mnemonic.into_descriptors(None, network, address_type),
            Self::Xpriv(xprv) => {
                let descriptor_secret_key =
                    DescriptorSecretKey::from_xpriv(network, xprv.to_xpriv());
                let new_descriptor = match address_type {
                    WalletAddressType::NativeSegwit => Descriptor::new_bip84,
                    WalletAddressType::WrappedSegwit => Descriptor::new_bip49,
                    WalletAddressType::Legacy => Descriptor::new_bip44,
                };
                let external = new_descriptor(
                    &descriptor_secret_key,
                    bdk_wallet::KeychainKind::External,
                    network,
                );
                let internal = new_descriptor(
                    &descriptor_secret_key,
                    bdk_wallet::KeychainKind::Internal,
                    network,
                );

                Descriptors { external, internal }
            }
        }
    }

    fn xpub(&self, network: Network) -> Xpub {
        match self {
            Self::Mnemonic(mnemonic) => mnemonic.xpub(BitcoinNetwork::from(network)),
            Self::Xpriv(xprv) => {
                let mut xpriv = xprv.to_xpriv();
                xpriv.network = BitcoinNetwork::from(network).into();

                Xpub::from_priv(&Secp256k1::new(), &xpriv)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::{NetworkKind, bip32::Xpriv};
    use cove_device::keychain::WalletXprv;

    use super::*;

    #[test]
    fn xpriv_uses_the_target_network_for_public_keys_and_descriptors() {
        let xpriv = Xpriv::new_master(BitcoinNetwork::Bitcoin, &[17; 32]).unwrap();
        let secret = WalletSecret::Xpriv(WalletXprv::from(xpriv));

        assert_eq!(secret.xpub(Network::Bitcoin).network, NetworkKind::Main);
        assert_eq!(secret.xpub(Network::Signet).network, NetworkKind::Test);

        let descriptors = secret.into_descriptors(Network::Signet, WalletAddressType::NativeSegwit);
        let create_params = descriptors.into_create_params().network(BitcoinNetwork::Signet);

        create_params.create_wallet_no_persist().unwrap();
    }
}
