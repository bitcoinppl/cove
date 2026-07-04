use std::sync::Arc;

use flume::{Receiver, Sender};
use tracing::error;

use crate::manager::deferred_sender::{DebugSend, DeferredSender, MessageSender, SingleOrMany};

/// Shared reconcile-channel plumbing used by every manager
///
/// Owns a bounded flume channel whose send side is a [`MessageSender`] and whose
/// receive side is drained by a single listener that forwards each message to the
/// FFI reconciler
#[derive(Debug)]
pub struct ReconcileChannel<M: DebugSend> {
    sender: MessageSender<M>,
    receiver: Arc<Receiver<SingleOrMany<M>>>,
}

impl<M: DebugSend> Clone for ReconcileChannel<M> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone(), receiver: self.receiver.clone() }
    }
}

impl<M: DebugSend> ReconcileChannel<M> {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = flume::bounded(capacity);
        Self { sender: MessageSender::new(sender), receiver: Arc::new(receiver) }
    }

    pub fn send(&self, message: impl Into<SingleOrMany<M>>) {
        self.sender.send(message);
    }

    /// Blocking send on the raw flume sender: preserves strict FIFO with no tokio
    /// dependency, for managers that predate the async-fallback [`send`](Self::send)
    pub fn send_sync(&self, message: impl Into<SingleOrMany<M>>) {
        if let Err(err) = self.sender.raw().send(message.into()) {
            error!("unable to send reconcile message: {err}");
        }
    }

    pub async fn send_async(&self, message: impl Into<SingleOrMany<M>>) {
        self.sender.send_async(message).await;
    }

    pub fn sender(&self) -> &MessageSender<M> {
        &self.sender
    }

    pub fn deferred_sender(&self) -> DeferredSender<M> {
        DeferredSender::new(self.sender.clone())
    }

    /// Clone the underlying flume sender to share with actors that hold a raw
    /// `Sender<SingleOrMany<M>>` and feed the same channel
    pub fn raw_sender(&self) -> Sender<SingleOrMany<M>> {
        self.sender.raw()
    }

    #[cfg(test)]
    pub fn receiver(&self) -> Arc<Receiver<SingleOrMany<M>>> {
        self.receiver.clone()
    }

    /// Spawn a dedicated OS thread that forwards each received message to `handler`
    pub fn listen(&self, mut handler: impl FnMut(SingleOrMany<M>) + Send + 'static) {
        let receiver = self.receiver.clone();
        std::thread::spawn(move || {
            while let Ok(field) = receiver.recv() {
                handler(field);
            }
        });
    }

    /// Spawn a tokio task that forwards each received message to `handler`
    pub fn listen_async(&self, mut handler: impl FnMut(SingleOrMany<M>) + Send + 'static) {
        let receiver = self.receiver.clone();
        cove_tokio::task::spawn(async move {
            while let Ok(field) = receiver.recv_async().await {
                handler(field);
            }
        });
    }
}
