use crossbeam::channel::Sender;
use once_cell::sync::OnceCell;

use crate::router::Router;

#[derive(uniffi::Enum)]
pub enum Update {
    RouterUpdate { router: Router },
}

pub static UPDATER: OnceCell<Updater> = OnceCell::new();
pub struct Updater(pub Sender<Update>);

impl Updater {
    /// Initialize global instance of the updater with a sender
    pub fn init(sender: Sender<Update>) {
        UPDATER.get_or_init(|| Updater(sender));
    }

    pub fn send_update(update: Update) {
        UPDATER
            .get()
            .expect("updater is not initialized")
            .0
            .send(update)
            .expect("failed to send update");
    }
}

#[uniffi::export(callback_interface)]
pub trait FfiUpdater: Send + Sync + 'static {
    /// Essentially a callback to the frontend
    fn update(&self, update: Update);
}
