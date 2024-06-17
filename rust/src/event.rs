use crate::router::Route;

#[derive(uniffi::Enum)]
pub enum Event {
    SetRoute { route: Route },
}
