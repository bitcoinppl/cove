/// 1) One generic enum to replace all of your `Messages` enums.
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

pub trait MessageManager<T>: Clone + Send + Sync + 'static {
    fn send(&self, msgs: SingleOrMany<T>);
}

pub struct DeferredSender<M, T>
where
    M: MessageManager<T>,
{
    manager: M,
    buffer: Vec<T>,
}

impl<M, T> DeferredSender<M, T>
where
    M: MessageManager<T>,
{
    pub fn new(manager: M) -> Self {
        DeferredSender { manager, buffer: Vec::new() }
    }

    pub fn queue(&mut self, msg: T) {
        self.buffer.push(msg);
    }
}

impl<M, T> Drop for DeferredSender<M, T>
where
    M: MessageManager<T>,
{
    fn drop(&mut self) {
        let msgs = std::mem::take(&mut self.buffer);
        if !msgs.is_empty() {
            self.manager.send(SingleOrMany::Many(msgs));
        }
    }
}
