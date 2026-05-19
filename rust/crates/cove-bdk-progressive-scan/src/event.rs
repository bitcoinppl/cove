use std::collections::BTreeMap;

use bdk_wallet::chain::{
    CheckPoint, ConfirmationBlockTime, TxUpdate, spk_client::FullScanResponse,
};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, ScanProgress};

#[derive(Debug)]
pub enum ScanEvent<K> {
    Progress(ScanProgress<K>),
    Update(ScanUpdate<K>),
    Complete(FullScanResponse<K>),
}

#[derive(Debug)]
pub struct ScanUpdate<K> {
    pub chain_update: Option<CheckPoint>,
    pub tx_update: TxUpdate<ConfirmationBlockTime>,
    pub last_active_indices: BTreeMap<K, u32>,
}

impl<K> Default for ScanUpdate<K> {
    fn default() -> Self {
        Self {
            chain_update: None,
            tx_update: TxUpdate::default(),
            last_active_indices: BTreeMap::new(),
        }
    }
}

impl<K> ScanUpdate<K> {
    pub fn is_empty(&self) -> bool {
        self.chain_update.is_none()
            && self.tx_update.is_empty()
            && self.last_active_indices.is_empty()
    }
}

pub(crate) fn clone_full_scan_response<K: Clone>(
    response: &FullScanResponse<K>,
) -> FullScanResponse<K> {
    FullScanResponse {
        tx_update: response.tx_update.clone(),
        last_active_indices: response.last_active_indices.clone(),
        chain_update: response.chain_update.clone(),
    }
}

pub(crate) fn send_progress<K>(events: &flume::Sender<ScanEvent<K>>, progress: ScanProgress<K>) {
    let _ = events.try_send(ScanEvent::Progress(progress));
}

pub(crate) fn send_update<K>(
    events: &flume::Sender<ScanEvent<K>>,
    update: ScanUpdate<K>,
) -> Result<()> {
    events.send(ScanEvent::Update(update)).map_err(|_| Error::ChannelClosed)
}

pub(crate) async fn send_update_async<K>(
    events: &flume::Sender<ScanEvent<K>>,
    update: ScanUpdate<K>,
) -> Result<()> {
    events.send_async(ScanEvent::Update(update)).await.map_err(|_| Error::ChannelClosed)
}

pub(crate) fn send_complete<K>(
    events: &flume::Sender<ScanEvent<K>>,
    response: FullScanResponse<K>,
) -> Result<()> {
    events.send(ScanEvent::Complete(response)).map_err(|_| Error::ChannelClosed)
}

pub(crate) fn send_complete_unless_cancelled<K>(
    events: &flume::Sender<ScanEvent<K>>,
    cancel_token: &CancellationToken,
    response: FullScanResponse<K>,
) -> Result<()> {
    if cancel_token.is_cancelled() {
        return Err(Error::Cancelled);
    }

    send_complete(events, response)
}

pub(crate) async fn send_complete_async<K>(
    events: &flume::Sender<ScanEvent<K>>,
    response: FullScanResponse<K>,
) -> Result<()> {
    events.send_async(ScanEvent::Complete(response)).await.map_err(|_| Error::ChannelClosed)
}

pub(crate) async fn send_complete_async_unless_cancelled<K>(
    events: &flume::Sender<ScanEvent<K>>,
    cancel_token: &CancellationToken,
    response: FullScanResponse<K>,
) -> Result<()> {
    if cancel_token.is_cancelled() {
        return Err(Error::Cancelled);
    }

    send_complete_async(events, response).await
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bdk_wallet::KeychainKind;
    use bdk_wallet::chain::spk_client::FullScanResponse;
    use bdk_wallet::test_utils::{get_test_wpkh_and_change_desc, new_wallet_and_funding_update};

    use crate::event::{
        send_complete, send_complete_async, send_complete_async_unless_cancelled,
        send_complete_unless_cancelled, send_progress, send_update, send_update_async,
    };
    use crate::{Error, ScanEvent, ScanProgress, ScanUpdate};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn progress_send_failure_does_not_fail_scan() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(0);
        drop(rx);

        send_progress(
            &tx,
            ScanProgress { keychain: KeychainKind::External, checked: 1, gap: 1, stop_gap: 20 },
        );
    }

    #[test]
    fn update_send_fails_when_receiver_is_closed() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        drop(rx);

        let result = send_update(&tx, ScanUpdate::default());

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn complete_send_fails_when_receiver_is_closed() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        drop(rx);

        let result = send_complete(&tx, FullScanResponse::default());

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn async_update_send_fails_when_receiver_is_closed() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        drop(rx);

        let result = futures::executor::block_on(send_update_async(&tx, ScanUpdate::default()));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn async_complete_send_fails_when_receiver_is_closed() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        drop(rx);

        let result =
            futures::executor::block_on(send_complete_async(&tx, FullScanResponse::default()));

        assert!(matches!(result, Err(Error::ChannelClosed)));
    }

    #[test]
    fn complete_send_checks_cancellation_before_sending() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let result =
            send_complete_unless_cancelled(&tx, &cancel_token, FullScanResponse::default());

        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(rx.try_iter().next().is_none());
    }

    #[test]
    fn async_complete_send_checks_cancellation_before_sending() {
        let (tx, rx) = flume::bounded::<ScanEvent<KeychainKind>>(1);
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        let result = futures::executor::block_on(send_complete_async_unless_cancelled(
            &tx,
            &cancel_token,
            FullScanResponse::default(),
        ));

        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(rx.try_iter().next().is_none());
    }

    #[test]
    fn partial_update_then_final_complete_is_idempotent() {
        let (external_descriptor, internal_descriptor) = get_test_wpkh_and_change_desc();
        let (mut partial_then_final, txid, update) =
            new_wallet_and_funding_update(external_descriptor, Some(internal_descriptor));
        let (mut final_only, _, _) =
            new_wallet_and_funding_update(external_descriptor, Some(internal_descriptor));
        let partial_response = FullScanResponse {
            chain_update: update.chain.clone(),
            tx_update: update.tx_update.clone(),
            last_active_indices: BTreeMap::new(),
        };
        let final_response_after_partial = FullScanResponse {
            chain_update: update.chain.clone(),
            tx_update: update.tx_update.clone(),
            last_active_indices: BTreeMap::from([
                (KeychainKind::External, 0),
                (KeychainKind::Internal, 0),
            ]),
        };
        let final_response_only = FullScanResponse {
            chain_update: update.chain,
            tx_update: update.tx_update,
            last_active_indices: BTreeMap::from([
                (KeychainKind::External, 0),
                (KeychainKind::Internal, 0),
            ]),
        };

        partial_then_final.apply_update(partial_response).unwrap();
        partial_then_final.apply_update(final_response_after_partial).unwrap();
        final_only.apply_update(final_response_only).unwrap();

        assert_eq!(partial_then_final.balance(), final_only.balance());
        assert_eq!(partial_then_final.transactions().count(), final_only.transactions().count());
        assert!(partial_then_final.get_tx(txid).is_some());
    }
}
