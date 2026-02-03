/// Contains the logic for the header icon presentation
use cove_macros::impl_default_for;

#[derive(Clone, uniffi::Object)]
pub struct HeaderIconPresenter {}

impl_default_for!(HeaderIconPresenter);

use crate::{
    color::{FfiColor, FfiOpacity},
    color_scheme::FfiColorScheme,
    transaction::{TransactionDirection, TransactionState},
};

#[uniffi::export]
impl HeaderIconPresenter {
    #[uniffi::constructor]
    pub const fn new() -> Self {
        Self {}
    }

    #[uniffi::method]
    pub const fn ring_color(
        &self,
        state: TransactionState,
        color_scheme: FfiColorScheme,
        direction: TransactionDirection,
        confirmations: i64,
        ring_number: i64,
    ) -> FfiColor {
        type S = TransactionState;
        type C = FfiColor;
        type X = FfiColorScheme;
        type D = TransactionDirection;

        let d = FfiOpacity(100);

        match (state, direction, color_scheme, confirmations, ring_number) {
            (S::Pending, _, X::Dark, _, _) => C::White(d),
            (S::Pending, _, X::Light, _, _) => C::CoolGray(d),
            (S::Confirmed, D::Outgoing, X::Dark, _, _) => C::White(d),
            (S::Confirmed, D::Outgoing, X::Light, _, _) => C::Black(d),
            (S::Confirmed, D::Incoming, X::Dark, _, 3) => C::Green(d),
            (S::Confirmed, D::Incoming, X::Dark, confirmations, 2) => {
                if confirmations > 1 {
                    C::Green(d)
                } else {
                    C::White(d)
                }
            }
            (S::Confirmed, D::Incoming, X::Dark, confirmations, 1) => {
                if confirmations > 2 {
                    C::Green(d)
                } else {
                    C::White(d)
                }
            }
            (S::Confirmed, D::Incoming, X::Light, confirmations, 3) => {
                if confirmations > 0 {
                    C::Green(d)
                } else {
                    C::Gray(d)
                }
            }
            (S::Confirmed, D::Incoming, X::Light, confirmations, 2) => {
                if confirmations > 1 {
                    C::Green(d)
                } else {
                    C::Gray(d)
                }
            }
            (S::Confirmed, D::Incoming, X::Light, confirmations, 1) => {
                if confirmations > 2 {
                    C::Green(d)
                } else {
                    C::Gray(d)
                }
            }
            (TransactionState::Confirmed, D::Incoming, _, _, _) => C::Green(d),
        }
    }

    #[uniffi::method]
    pub const fn icon_color(
        &self,
        state: TransactionState,
        direction: TransactionDirection,
        color_scheme: FfiColorScheme,
        confirmation_count: i64,
    ) -> FfiColor {
        type S = TransactionState;
        type C = FfiColor;
        type X = FfiColorScheme;
        type D = TransactionDirection;

        let d = FfiOpacity(100);
        let a50 = FfiOpacity(50);
        let a80 = FfiOpacity(80);

        match (state, direction, color_scheme, confirmation_count) {
            (S::Confirmed, D::Incoming, X::Dark, 1) => C::Green(a50),
            (S::Confirmed, D::Incoming, X::Dark, 2) => C::Green(a80),
            (S::Confirmed, D::Incoming, X::Dark, _) => C::Green(d),
            (S::Confirmed, D::Incoming, X::Light, _) => C::White(d),
            (S::Confirmed, D::Outgoing, _, 1) => C::White(a50),
            (S::Confirmed, D::Outgoing, _, 2) => C::White(a80),
            (S::Confirmed, D::Outgoing, _, _) => C::White(d),
            (S::Pending, _, X::Light, _) => C::Black(a50),
            (S::Pending, _, X::Dark, _) => C::White(d),
        }
    }

    #[uniffi::method]
    const fn background_color(
        &self,
        state: TransactionState,
        direction: TransactionDirection,
        color_scheme: FfiColorScheme,
        confirmation_count: i64,
    ) -> FfiColor {
        type S = TransactionState;
        type C = FfiColor;
        type X = FfiColorScheme;
        type D = TransactionDirection;

        let d = FfiOpacity(100);
        let a33 = FfiOpacity(33);
        let a55 = FfiOpacity(55);

        match (state, direction, color_scheme, confirmation_count) {
            (S::Pending, _, X::Dark, _) => C::Black(d),
            (S::Pending, _, X::Light, _) => C::CoolGray(d),
            (S::Confirmed, D::Incoming, X::Light, 1) => C::Green(a33),
            (S::Confirmed, D::Incoming, X::Light, 2) => C::Green(a55),
            (S::Confirmed, D::Incoming, X::Light, _) => C::Green(d),
            (S::Confirmed, D::Outgoing, X::Light, 1) => C::Black(a33),
            (S::Confirmed, D::Outgoing, X::Light, 2) => C::Black(a55),
            (S::Confirmed, D::Outgoing, X::Light, _) => C::Black(d),
            (S::Confirmed, _, X::Dark, _) => C::Black(d),
        }
    }
}
