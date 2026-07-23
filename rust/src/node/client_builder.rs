use crate::node::client::Error;

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
}
