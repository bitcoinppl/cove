use bdk_electrum::{BdkElectrumClient, electrum_client::ElectrumApi};
use bdk_esplora::esplora_client::{self, Sleeper};
use bdk_wallet::chain::spk_client::FullScanRequest;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, ScanEvent};
use crate::{ProgressiveElectrumScanner, ProgressiveEsploraScanner};

pub struct ProgressiveScanner<K> {
    request: FullScanRequest<K>,
    stop_gap: usize,
    events: flume::Sender<ScanEvent<K>>,
    cancel_token: CancellationToken,
    last_revealed_indices: BTreeMap<K, u32>,
}

pub(crate) struct ProgressiveScannerParts<K> {
    pub request: FullScanRequest<K>,
    pub stop_gap: usize,
    pub events: flume::Sender<ScanEvent<K>>,
    pub cancel_token: CancellationToken,
    pub last_revealed_indices: BTreeMap<K, u32>,
}

impl<K> ProgressiveScanner<K> {
    pub fn builder() -> ProgressiveScannerBuilder<K> {
        ProgressiveScannerBuilder::default()
    }

    pub fn request(&self) -> &FullScanRequest<K> {
        &self.request
    }

    pub fn stop_gap(&self) -> usize {
        self.stop_gap
    }

    pub fn events(&self) -> &flume::Sender<ScanEvent<K>> {
        &self.events
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }

    pub fn last_revealed_indices(&self) -> &BTreeMap<K, u32> {
        &self.last_revealed_indices
    }

    pub(crate) fn into_parts(self) -> ProgressiveScannerParts<K> {
        ProgressiveScannerParts {
            request: self.request,
            stop_gap: self.stop_gap,
            events: self.events,
            cancel_token: self.cancel_token,
            last_revealed_indices: self.last_revealed_indices,
        }
    }
}

pub struct ProgressiveScannerBuilder<K> {
    request: Option<FullScanRequest<K>>,
    stop_gap: Option<usize>,
    events: Option<flume::Sender<ScanEvent<K>>>,
    cancel_token: Option<CancellationToken>,
    last_revealed_indices: BTreeMap<K, u32>,
}

impl<K> Default for ProgressiveScannerBuilder<K> {
    fn default() -> Self {
        Self {
            request: None,
            stop_gap: None,
            events: None,
            cancel_token: None,
            last_revealed_indices: BTreeMap::new(),
        }
    }
}

impl<K> ProgressiveScannerBuilder<K> {
    pub fn request(mut self, request: FullScanRequest<K>) -> Self {
        self.request = Some(request);
        self
    }

    pub fn stop_gap(mut self, stop_gap: usize) -> Self {
        self.stop_gap = Some(stop_gap);
        self
    }

    pub fn events(mut self, events: flume::Sender<ScanEvent<K>>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn cancel_token(mut self, cancel_token: CancellationToken) -> Self {
        self.cancel_token = Some(cancel_token);
        self
    }

    pub fn last_revealed_indices(mut self, indices: BTreeMap<K, u32>) -> Self {
        self.last_revealed_indices = indices;
        self
    }

    pub fn build(self) -> Result<ProgressiveScanner<K>> {
        Ok(ProgressiveScanner {
            request: self.request.ok_or(Error::MissingRequest)?,
            stop_gap: self.stop_gap.ok_or(Error::MissingStopGap)?,
            events: self.events.ok_or(Error::MissingEvents)?,
            cancel_token: self.cancel_token.unwrap_or_default(),
            last_revealed_indices: self.last_revealed_indices,
        })
    }

    pub fn esplora<S>(
        self,
        client: impl Into<Arc<esplora_client::AsyncClient<S>>>,
    ) -> Result<ProgressiveEsploraScanner<K, S>>
    where
        K: Ord + Clone + Send,
        S: Sleeper + Clone + Send + Sync,
        S::Sleep: Send,
    {
        Ok(ProgressiveEsploraScanner::new(self.build()?, client))
    }

    pub fn electrum<E>(
        self,
        client: impl Into<Arc<BdkElectrumClient<E>>>,
    ) -> Result<ProgressiveElectrumScanner<K, E>>
    where
        K: Ord + Clone,
        E: ElectrumApi,
    {
        Ok(ProgressiveElectrumScanner::new(self.build()?, client))
    }
}

#[cfg(test)]
mod tests {
    use bdk_wallet::KeychainKind;
    use bdk_wallet::chain::spk_client::FullScanRequest;

    use crate::{Error, ProgressiveScanner, ScanEvent};

    #[test]
    fn builder_requires_request() {
        let (tx, _) = flume::bounded::<ScanEvent<KeychainKind>>(1);

        let result = ProgressiveScanner::builder().stop_gap(1).events(tx).build();

        assert!(matches!(result, Err(Error::MissingRequest)));
    }

    #[test]
    fn builder_requires_stop_gap() {
        let (tx, _) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        let request = FullScanRequest::builder().build();

        let result = ProgressiveScanner::builder().request(request).events(tx).build();

        assert!(matches!(result, Err(Error::MissingStopGap)));
    }

    #[test]
    fn builder_requires_events() {
        let request = FullScanRequest::<KeychainKind>::builder().build();

        let result = ProgressiveScanner::builder().request(request).stop_gap(1).build();

        assert!(matches!(result, Err(Error::MissingEvents)));
    }
}
