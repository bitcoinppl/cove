//! The fake Esplora and Electrum node from `cove-fake-node`, plus the helpers
//! that point Cove's node settings at it
//!
//! [`FakeNode`] derefs to the crate's node, so tests reach the chain, request
//! log and scripted rules directly

use std::ops::{Deref, DerefMut};

use bitcoin::Network;

use crate::{database::Database, network::Network as CoveNetwork, node::Node};

pub(crate) use cove_fake_node::{Protocol, unreachable_url, wait_until};

#[cfg(test)]
mod tests;

/// A fake Esplora and Electrum server pair that knows Cove's node types
pub(crate) struct FakeNode(cove_fake_node::FakeNode);

impl FakeNode {
    /// Starts both servers on a chain that holds only the genesis block of `network`
    pub(crate) fn start(network: Network) -> Self {
        Self(cove_fake_node::FakeNode::start(network))
    }

    pub(crate) fn esplora_node(&self, name: &str) -> Node {
        Node::new_esplora(name.to_string(), self.esplora_url(), self.cove_network())
    }

    pub(crate) fn electrum_node(&self, name: &str) -> Node {
        Node::new_electrum(name.to_string(), self.electrum_url(), self.cove_network())
    }

    /// Makes the wallet code connect to this node's Esplora server
    pub(crate) fn select_esplora(&self) -> Node {
        self.select(self.esplora_node("fake esplora"))
    }

    /// Makes the wallet code connect to this node's Electrum server
    pub(crate) fn select_electrum(&self) -> Node {
        self.select(self.electrum_node("fake electrum"))
    }

    fn select(&self, node: Node) -> Node {
        Database::global()
            .global_config
            .set_selected_node(&node)
            .expect("the fake node can be selected");

        node
    }

    fn cove_network(&self) -> CoveNetwork {
        CoveNetwork::try_from(self.chain().network()).expect("fake node network is supported")
    }
}

impl Deref for FakeNode {
    type Target = cove_fake_node::FakeNode;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FakeNode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
