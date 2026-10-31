use std::{collections::VecDeque, fmt::Debug, sync::Arc};

use flume::{Sender, TrySendError};
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
        let message = message.into();
        trace!("send: {message:?}");

        let mut overflow = self.overflow.lock();
        if !overflow.is_idle() {
            overflow.queue.push_back(message);
            return;
        }

        match self.sender.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(message)) => {
                warn!("[WARN] unable to send, queue is full, sending async");
                overflow.queue.push_back(message);
                overflow.draining = true;

                let me = self.clone();
                task::spawn(async move { me.drain_overflow().await });
            }
            Err(e) => {
                error!("unable to send message to send flow manager: {e:?}");
            }
        }
    }

    /// Waits for room in the channel, unless earlier messages are already waiting, in which case
    /// the message joins them and is sent after them
    pub async fn send_async(&self, message: impl Into<SingleOrMany<T>>) {
        let message = message.into();
        trace!("send_async: {message:?}");

        {
            let mut overflow = self.overflow.lock();
            if !overflow.is_idle() {
                overflow.queue.push_back(message);
                return;
            }
        }

        if let Err(err) = self.sender.send_async(message).await {
            error!("unable to send message to send flow manager: {err}");
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
                error!("unable to send message to send flow manager: {err}");

                // the receiver is gone, so nothing queued can be delivered either
                let mut overflow = self.overflow.lock();
                overflow.queue.clear();
                overflow.draining = false;
                return;
            }
        }
    }

    /// Clone the underlying flume sender to share with actors that hold a raw
    /// `Sender<SingleOrMany<T>>` and feed the same channel
    pub fn raw(&self) -> Sender<SingleOrMany<T>> {
        self.sender.clone()
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
