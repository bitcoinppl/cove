use std::fmt;

use cove_types::Network;

mod template;

pub use template::{CustomBlockExplorerError, CustomBlockExplorerTemplate};

pub const PREVIEW_TXID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BlockExplorerOption {
    MempoolSpace,
    MempoolGuide,
    BullBitcoin,
    Blockstream,
    Custom,
}

#[uniffi::export]
impl BlockExplorerOption {
    /// Returns the user-visible label for this explorer option.
    pub fn display_name(&self) -> String {
        self.as_display_name().to_string()
    }
}

impl BlockExplorerOption {
    const PRESETS: [Self; 5] = [
        Self::MempoolSpace,
        Self::MempoolGuide,
        Self::BullBitcoin,
        Self::Blockstream,
        Self::Custom,
    ];

    /// Returns all options in the order shown by the settings UI.
    pub(crate) const fn all() -> [Self; 5] {
        Self::PRESETS
    }

    /// Returns the base URL for preset options that are backed by stored custom templates.
    pub(crate) const fn base_url(&self) -> Option<&'static str> {
        match self {
            Self::MempoolSpace | Self::Custom => None,
            Self::MempoolGuide => Some("https://mempool.guide/"),
            Self::BullBitcoin => Some("https://mempool.bullbitcoin.com/"),
            Self::Blockstream => Some("https://blockstream.info/"),
        }
    }

    /// Returns the option represented by a stored template, falling back to the default option.
    pub(crate) fn matching_stored_template(
        network: Network,
        stored_template: Option<&str>,
    ) -> Self {
        let Some(stored_template) = stored_template else {
            return Self::MempoolSpace;
        };

        let Ok(template) = CustomBlockExplorerTemplate::parse_stored(stored_template) else {
            return Self::MempoolSpace;
        };

        Self::all()
            .into_iter()
            .find(|option| option.matches_template(network, &template))
            .unwrap_or(Self::Custom)
    }

    /// Returns the static display name for this option.
    fn as_display_name(&self) -> &'static str {
        match self {
            Self::MempoolSpace => "Default (mempool.space)",
            Self::MempoolGuide => "mempool.guide",
            Self::BullBitcoin => "mempool.bullbitcoin.com",
            Self::Blockstream => "blockstream.info",
            Self::Custom => "Custom",
        }
    }

    /// Builds the canonical transaction template for this option on the given network.
    fn template_for_network(&self, network: Network) -> Option<CustomBlockExplorerTemplate> {
        match self {
            Self::MempoolSpace => Some(CustomBlockExplorerTemplate::default_for(network)),
            Self::Custom => None,
            _ => self.base_url().and_then(|base_url| {
                CustomBlockExplorerTemplate::from_preset_base_url(network, base_url)
            }),
        }
    }

    /// Returns whether a stored template exactly matches this preset option.
    fn matches_template(&self, network: Network, template: &CustomBlockExplorerTemplate) -> bool {
        self.template_for_network(network)
            .is_some_and(|preset_template| preset_template.as_str() == template.as_str())
    }
}

#[uniffi::export]
/// Returns every block explorer option exposed to mobile clients.
pub fn all_block_explorer_options() -> Vec<BlockExplorerOption> {
    BlockExplorerOption::all().to_vec()
}

/// Returns the transaction URL for a stored template or the network default.
pub fn effective_transaction_url(
    network: Network,
    stored_template: Option<&str>,
    txid: impl fmt::Display,
) -> String {
    let Some(stored_template) = stored_template else {
        return CustomBlockExplorerTemplate::default_for(network).render(txid);
    };

    match CustomBlockExplorerTemplate::parse_stored(stored_template) {
        Ok(template) => template.render(txid),
        Err(_) => CustomBlockExplorerTemplate::default_for(network).render(txid),
    }
}

#[cfg(test)]
mod tests {
    use super::BlockExplorerOption;

    #[test]
    fn block_explorer_options_expose_expected_order_and_labels() {
        let options = super::all_block_explorer_options();

        assert_eq!(
            options.as_slice(),
            &[
                BlockExplorerOption::MempoolSpace,
                BlockExplorerOption::MempoolGuide,
                BlockExplorerOption::BullBitcoin,
                BlockExplorerOption::Blockstream,
                BlockExplorerOption::Custom,
            ]
        );
        assert_eq!(BlockExplorerOption::MempoolSpace.display_name(), "Default (mempool.space)");
        assert_eq!(BlockExplorerOption::Blockstream.display_name(), "blockstream.info");
    }
}
