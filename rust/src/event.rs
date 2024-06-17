use crate::router::Route;

// events.rs
#[derive(uniffi::Enum)]
pub enum Event {
    SetRoute { route: Route },
}
