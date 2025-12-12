//! Deferred dispatch pattern for safely dispatching actions from Rust code.
//!
//! When dispatching actions (like `AppAction`) from Rust, calling dispatch directly
//! can cause deadlocks if any locks are held at the call site. `DeferredDispatch<T>`
//! solves this by queuing actions and dispatching them when the struct is dropped,
//! guaranteeing dispatch happens after the function scope ends and all locks are released.
//!
//! # Example
//!
//! ```ignore
//! fn some_function() {
//!     let mut deferred = DeferredDispatch::<AppAction>::new();
//!     deferred.queue(AppAction::UpdateFees);
//!     deferred.queue(AppAction::UpdateFiatPrices);
//!
//!     // ... do work, potentially holding locks ...
//!
//!     // dispatch happens automatically here when deferred goes out of scope
//! }
//! ```
//!
//! # Adding support for new action types
//!
//! Implement the `Dispatchable` trait for your action type:
//!
//! ```ignore
//! impl Dispatchable for MyAction {
//!     fn flush(actions: Vec<Self>) {
//!         for action in actions {
//!             // dispatch logic
//!         }
//!     }
//! }
//! ```

use std::fmt::Debug;

/// Trait for action types that can be dispatched in a deferred manner
/// Implement this for any action type that should support deferred dispatch
pub trait Dispatchable: Debug + Sized {
    fn flush(actions: Vec<Self>);
}

/// Queues actions and dispatches them when dropped
/// Use this instead of calling dispatch directly from Rust code to avoid potential deadlocks
#[derive(Debug)]
pub struct DeferredDispatch<T: Dispatchable> {
    actions: Vec<T>,
}

impl<T: Dispatchable> DeferredDispatch<T> {
    pub fn new() -> Self {
        Self { actions: vec![] }
    }

    pub fn queue(&mut self, action: T) {
        self.actions.push(action);
    }
}

impl<T: Dispatchable> Default for DeferredDispatch<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Dispatchable> Drop for DeferredDispatch<T> {
    fn drop(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        if !actions.is_empty() && !std::thread::panicking() {
            T::flush(actions);
        }
    }
}
