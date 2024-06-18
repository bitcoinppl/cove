//! Event types that the frontend can send to the rust app

use crate::router::Route;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
#[allow(clippy::enum_variant_names)]
pub enum Event {
    RouteChanged { routes: Vec<Route> },
}
