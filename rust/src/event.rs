//! Event types that the frontend can send to the rust app
//! MainViewModel event

use crate::router::Route;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
#[allow(clippy::enum_variant_names)]
pub enum Event {
    RouteChanged { routes: Vec<Route> },
}

