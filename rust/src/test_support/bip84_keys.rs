//! BIP 84 keys and addresses built straight from BDK, independent of how Cove stores wallets

use bdk_wallet::{
    KeychainKind,
    bitcoin::bip32::Xpriv,
    descriptor::ExtendedDescriptor,
    template::{Bip84, DescriptorTemplate as _},
};
use bip39::Mnemonic;
use cove_types::address::AddressWithNetwork;

use crate::network::Network;

/// A fresh key set, so wallets other tests leave behind can never own its addresses
pub(crate) fn random_mnemonic() -> Mnemonic {
    Mnemonic::from_entropy(&rand::random::<[u8; 16]>()).unwrap()
}

/// BIP 84 account 0 descriptors built straight from BDK, independent of how Cove stores them
pub(crate) fn bip84_descriptor(
    mnemonic: &Mnemonic,
    network: Network,
    keychain: KeychainKind,
) -> ExtendedDescriptor {
    let network = bitcoin::Network::from(network);
    let xprv = Xpriv::new_master(network, &mnemonic.to_seed("")).unwrap();
    let (descriptor, _, _) = Bip84(xprv, keychain).build(network.into()).unwrap();

    descriptor
}

pub(crate) fn address_at(
    mnemonic: &Mnemonic,
    network: Network,
    keychain: KeychainKind,
    index: u32,
) -> AddressWithNetwork {
    let address = bip84_descriptor(mnemonic, network, keychain)
        .at_derivation_index(index)
        .unwrap()
        .address(bitcoin::Network::from(network))
        .unwrap();

    AddressWithNetwork::try_new(&address.to_string()).unwrap()
}
