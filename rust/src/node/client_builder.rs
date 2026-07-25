use std::time::Duration;

use crate::node::client::{Error, NodeClientOptions};

use super::{Node, client::NodeClient};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeClientBuilder {
    pub node: Node,
    pub batch_size: usize,
}
impl NodeClientBuilder {
    pub async fn build(&self) -> Result<NodeClient, Error> {
        let node_client = NodeClient::try_from_builder(self).await?;
        Ok(node_client)
    }

    /// Builds with a caller-chosen bound on waiting for built-in Tor readiness
    ///
    /// Use this when the caller's own deadline is shorter than the default bound
    pub(crate) async fn build_within(
        &self,
        tor_ready_bound: Duration,
    ) -> Result<NodeClient, Error> {
        let options =
            NodeClientOptions::from_db(self.batch_size)?.with_tor_ready_bound(tor_ready_bound);

        NodeClient::new_with_options(&self.node, options).await
    }
}
