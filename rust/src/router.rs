use crate::impl_default_for;

#[derive(Clone, uniffi::Enum)]
pub enum Route {
    Cove,
}

#[derive(Clone, uniffi::Record)]
pub struct Router {
    pub route: Route,
}

impl_default_for!(Router);
impl Router {
    pub fn new() -> Self {
        Self { route: Route::Cove }
    }
}
