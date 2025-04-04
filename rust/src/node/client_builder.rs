use crate::node::client::Error;

use super::{
    Node,
    client::{NodeClient, NodeClientOptions},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeClientBuilder {
    pub node: Node,
    pub options: NodeClientOptions,
}
impl NodeClientBuilder {
    pub async fn build(&self) -> Result<NodeClient, Error> {
        let node_client = NodeClient::try_from_builder(&self).await?;
        Ok(node_client)
    }

    pub async fn try_into_client(self) -> Result<NodeClient, Error> {
        let node_client = NodeClient::new_with_options(&self.node, self.options).await?;
        Ok(node_client)
    }
}
