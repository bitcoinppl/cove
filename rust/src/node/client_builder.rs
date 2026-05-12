use crate::{database::Database, node::client::Error};

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
    pub fn with_defaults(node: Node, batch_size: usize) -> Self {
        let db = Database::global();
        let config = db.global_config();
        let tor_external_host = config
            .tor_external_host()
            .ok()
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| "127.0.0.1".into());

        let options = NodeClientOptions {
            batch_size,
            use_tor: config.use_tor(),
            tor_mode: config.tor_mode().unwrap_or_default(),
            tor_external_host,
            tor_external_port: config.tor_external_port(),
        };

        Self { node, options }
    }

    pub async fn build(&self) -> Result<NodeClient, Error> {
        let node_client = NodeClient::try_from_builder(self).await?;
        Ok(node_client)
    }

    pub async fn try_into_client(self) -> Result<NodeClient, Error> {
        let node_client = NodeClient::new_with_options(&self.node, self.options).await?;
        Ok(node_client)
    }
}
