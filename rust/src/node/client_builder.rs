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
        NodeClient::new_with_options(&self.node, self.options).await
    }
}
