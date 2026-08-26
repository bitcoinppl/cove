use act_zero::{Actor, Addr, AddrLike as _};
use futures::{FutureExt as _, channel::oneshot, future::BoxFuture};
use tokio_util::sync::CancellationToken;

use crate::{
    discovery_scanner::WalletDiscoveryScanner, manager::wallet_manager::actor::WalletActor,
};

use super::TerminalPayjoinPersistenceAuthority;

enum TerminalReply {
    Completed(Result<(), String>),
    Cancelled,
}

pub(crate) async fn terminal_wallet_shutdown(
    authority: Option<TerminalPayjoinPersistenceAuthority>,
    actor: Addr<WalletActor>,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    terminal_shutdown(actor, deadline, move |actor| {
        async move {
            match authority {
                Some(authority) => actor.quiesce_for_terminal_shutdown(authority).await,
                None => actor.shutdown().await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())
        }
        .boxed()
    })
    .await
}

pub(crate) async fn terminal_discovery_shutdown(
    actor: Addr<WalletDiscoveryScanner>,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    terminal_shutdown(actor, deadline, |actor| {
        async move { actor.shutdown().await.map(|_| ()).map_err(|error| error.to_string()) }.boxed()
    })
    .await
}

pub(crate) async fn terminal_shutdown<T>(
    actor: Addr<T>,
    deadline: tokio::time::Instant,
    quiesce: impl for<'a> FnOnce(&'a mut T) -> BoxFuture<'a, Result<(), String>> + Send + 'static,
) -> Result<(), ()>
where
    T: Actor,
{
    if actor.termination().now_or_never().is_some() {
        return Ok(());
    }

    let cancellation = CancellationToken::new();
    let request_cancellation = cancellation.clone();
    let (started_sender, started_receiver) = oneshot::channel();
    let (reply_sender, reply_receiver) = oneshot::channel();
    actor.send_mut(Box::new(move |actor| {
        async move {
            let _ = started_sender.send(());
            let result = tokio::select! {
                biased;
                () = request_cancellation.cancelled() => TerminalReply::Cancelled,
                result = quiesce(actor) => {
                    if request_cancellation.is_cancelled() {
                        TerminalReply::Cancelled
                    } else {
                        TerminalReply::Completed(result)
                    }
                }
            };
            let terminate = matches!(result, TerminalReply::Completed(Ok(())));
            let _ = reply_sender.send(result);
            terminate
        }
        .boxed()
    }));

    let started = tokio::select! {
        result = started_receiver => result.is_ok(),
        () = tokio::time::sleep_until(deadline) => false,
    };
    if !started {
        cancellation.cancel();
        return Err(());
    }

    let mut reply_receiver = reply_receiver;
    let reply = tokio::select! {
        biased;
        result = &mut reply_receiver => result.ok(),
        () = tokio::time::sleep_until(deadline) => {
            cancellation.cancel();
            None
        }
    };

    match reply {
        Some(TerminalReply::Completed(Ok(()))) => {
            actor.termination().await;
            Ok(())
        }
        Some(TerminalReply::Completed(Err(_))) | Some(TerminalReply::Cancelled) | None => Err(()),
    }
}
