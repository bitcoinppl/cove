use std::{collections::VecDeque, fmt::Debug, sync::Arc};

use flume::{SendError, Sender, TrySendError};
use parking_lot::Mutex;
use tracing::{error, trace, warn};

use cove_tokio::task;

pub trait DebugSend: Debug + Send + Sync + 'static {}
impl<T> DebugSend for T where T: Debug + Send + Sync + 'static {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SingleOrMany<T> {
    Single(T),
    Many(Vec<T>),
}

impl<T> SingleOrMany<T> {
    /// The message type and count, for logs
    ///
    /// Payloads carry wallet names, UTXO labels, routes and typed text, so logs print this
    /// instead of the messages
    pub fn log_summary(&self) -> String {
        let type_name = std::any::type_name::<T>();
        let kind = type_name.rsplit("::").next().unwrap_or(type_name);

        match self {
            Self::Single(_) => kind.to_string(),
            Self::Many(messages) => format!("{} x {kind}", messages.len()),
        }
    }
}

impl<T> From<T> for SingleOrMany<T> {
    fn from(msg: T) -> Self {
        Self::Single(msg)
    }
}

impl<T> From<Vec<T>> for SingleOrMany<T> {
    fn from(msgs: Vec<T>) -> Self {
        Self::Many(msgs)
    }
}

#[derive(Debug, Clone)]
pub struct DeferredSender<T: DebugSend> {
    sender: MessageSender<T>,
    buffer: Vec<T>,
}

impl<T: DebugSend> DeferredSender<T> {
    pub const fn new(sender: MessageSender<T>) -> Self {
        Self { sender, buffer: vec![] }
    }

    pub fn queue(&mut self, message: T) {
        self.buffer.push(message);
    }
}

/// Sends reconcile messages in the order they were sent, even when the bounded channel is full
///
/// Each message is a delta that the UI applies over the last one, so a stale update that arrives
/// late overwrites newer state, such as a valid address replaced by the `None` from an earlier
/// keystroke. Clones share the overflow queue, so they keep one order between them
#[derive(Debug)]
pub struct MessageSender<T> {
    sender: Sender<SingleOrMany<T>>,
    overflow: Arc<Mutex<Overflow<T>>>,
}

/// Messages waiting for room in the channel, oldest first
#[derive(Debug)]
struct Overflow<T> {
    queue: VecDeque<SingleOrMany<T>>,
    /// Whether a drain task is forwarding the queue, including the message it holds while it
    /// waits for room, which is no longer in `queue`
    draining: bool,
}

impl<T> Overflow<T> {
    fn is_idle(&self) -> bool {
        self.queue.is_empty() && !self.draining
    }
}

impl<T> Clone for MessageSender<T> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone(), overflow: self.overflow.clone() }
    }
}

impl<T: DebugSend> MessageSender<T> {
    pub fn new(sender: Sender<SingleOrMany<T>>) -> Self {
        let overflow = Overflow { queue: VecDeque::new(), draining: false };
        Self { sender, overflow: Arc::new(Mutex::new(overflow)) }
    }

    pub fn send(&self, message: impl Into<SingleOrMany<T>>) {
        if let Err(err) = self.send_checked(message) {
            error!("unable to send reconcile message: {err}");
        }
    }

    /// Sends like [`send`](Self::send), but hands the message back when the receiver is gone, so
    /// a producer can stop work nobody will see
    pub fn send_checked(
        &self,
        message: impl Into<SingleOrMany<T>>,
    ) -> Result<(), SendError<SingleOrMany<T>>> {
        let message = message.into();
        trace!("send: {}", message.log_summary());

        let mut overflow = self.overflow.lock();
        if !overflow.is_idle() {
            if self.sender.is_disconnected() {
                return Err(SendError(message));
            }

            overflow.queue.push_back(message);
            return Ok(());
        }

        match self.sender.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(message)) => {
                warn!("[WARN] unable to send, queue is full, sending async");
                overflow.queue.push_back(message);
                overflow.draining = true;

                let me = self.clone();
                task::spawn(async move { me.drain_overflow().await });
                Ok(())
            }
            Err(TrySendError::Disconnected(message)) => Err(SendError(message)),
        }
    }

    /// Blocks for room in the channel, unless earlier messages are already waiting, in which case
    /// the message joins them and is sent after them
    ///
    /// Needs no tokio runtime unless [`send`](Self::send) already queued an overflow
    pub fn send_blocking(&self, message: impl Into<SingleOrMany<T>>) {
        let message = message.into();
        trace!("send_blocking: {}", message.log_summary());

        {
            let mut overflow = self.overflow.lock();
            if !overflow.is_idle() {
                overflow.queue.push_back(message);
                return;
            }
        }

        if let Err(err) = self.sender.send(message) {
            error!("unable to send reconcile message: {err}");
        }
    }

    /// Waits for room in the channel, unless earlier messages are already waiting, in which case
    /// the message joins them and is sent after them
    pub async fn send_async(&self, message: impl Into<SingleOrMany<T>>) {
        let message = message.into();
        trace!("send_async: {}", message.log_summary());

        {
            let mut overflow = self.overflow.lock();
            if !overflow.is_idle() {
                overflow.queue.push_back(message);
                return;
            }
        }

        if let Err(err) = self.sender.send_async(message).await {
            error!("unable to send reconcile message: {err}");
        }
    }

    /// Forwards the overflow queue one message at a time, so nothing sent later overtakes it
    async fn drain_overflow(&self) {
        loop {
            let message = {
                let mut overflow = self.overflow.lock();
                let Some(message) = overflow.queue.pop_front() else {
                    overflow.draining = false;
                    return;
                };

                message
            };

            if let Err(err) = self.sender.send_async(message).await {
                error!("unable to send reconcile message: {err}");

                // the receiver is gone, so nothing queued can be delivered either
                let mut overflow = self.overflow.lock();
                overflow.queue.clear();
                overflow.draining = false;
                return;
            }
        }
    }
}

impl<T: DebugSend> Drop for DeferredSender<T> {
    fn drop(&mut self) {
        let mut msgs = std::mem::take(&mut self.buffer);
        match msgs.len() {
            0 => {}
            1 => self.sender.send(SingleOrMany::Single(msgs.pop().expect("just checked len"))),
            _ => self.sender.send(SingleOrMany::Many(msgs)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SingleOrMany;

    #[derive(Debug)]
    struct WalletRenamed(String);

    #[test]
    fn log_summary_names_the_type_and_count_without_payloads() {
        let single = SingleOrMany::Single(WalletRenamed("abandon ability able".to_string()));
        let many = SingleOrMany::Many(vec![
            WalletRenamed("abandon".to_string()),
            WalletRenamed("ability".to_string()),
        ]);

        assert_eq!(single.log_summary(), "WalletRenamed");
        assert_eq!(many.log_summary(), "2 x WalletRenamed");
    }
}
