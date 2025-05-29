use std::fmt::Debug;

use flume::{Sender, TrySendError};
use tracing::{debug, error, warn};

use crate::task;

#[derive(Debug, Clone, PartialEq)]
pub enum SingleOrMany<T> {
    Single(T),
    Many(Vec<T>),
}

impl<T> From<T> for SingleOrMany<T> {
    fn from(msg: T) -> Self {
        SingleOrMany::Single(msg)
    }
}

impl<T> From<Vec<T>> for SingleOrMany<T> {
    fn from(msgs: Vec<T>) -> Self {
        SingleOrMany::Many(msgs)
    }
}

#[derive(Debug, Clone)]
pub struct DeferredSender<T>
where
    T: Debug + Send + Sync + 'static,
{
    sender: MessageSender<T>,
    buffer: Vec<T>,
}

impl<T> DeferredSender<T>
where
    T: Debug + Send + Sync + 'static,
{
    pub fn new(sender: MessageSender<T>) -> Self {
        Self { sender, buffer: vec![] }
    }

    pub fn queue(&mut self, message: T) {
        self.buffer.push(message);
    }
}

#[derive(Debug)]
pub struct MessageSender<T> {
    sender: Sender<SingleOrMany<T>>,
}

impl<T> Clone for MessageSender<T> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone() }
    }
}

impl<T> MessageSender<T>
where
    T: Debug + Send + Sync + 'static,
{
    pub fn new(sender: Sender<SingleOrMany<T>>) -> Self {
        Self { sender }
    }

    pub fn send(&self, message: impl Into<SingleOrMany<T>>) {
        let message = message.into();
        debug!("send: {message:?}");
        match self.sender.try_send(message) {
            Ok(_) => {}
            Err(TrySendError::Full(message)) => {
                warn!("[WARN] unable to send, queue is full, sending async");

                let me = self.clone();
                task::spawn(async move { me.send_async(message).await });
            }
            Err(e) => {
                error!("unable to send message to send flow manager: {e:?}");
            }
        }
    }

    pub fn try_send(
        &self,
        message: impl Into<SingleOrMany<T>>,
    ) -> Result<(), TrySendError<SingleOrMany<T>>> {
        self.sender.try_send(message.into())
    }

    pub async fn send_async(&self, message: impl Into<SingleOrMany<T>>) {
        let message = message.into();
        debug!("send_async: {message:?}");
        if let Err(err) = self.sender.send_async(message).await {
            error!("unable to send message to send flow manager: {err}");
        }
    }
}

impl<T> Drop for DeferredSender<T>
where
    T: Debug + Send + Sync + 'static,
{
    fn drop(&mut self) {
        let mut msgs = std::mem::take(&mut self.buffer);
        match msgs.len() {
            0 => {}
            1 => self.sender.send(SingleOrMany::Single(msgs.pop().expect("just checked len"))),
            _ => self.sender.send(SingleOrMany::Many(msgs)),
        }
    }
}
