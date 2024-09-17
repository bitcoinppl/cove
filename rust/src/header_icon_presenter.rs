#[derive(Clone, uniffi::Object)]
pub struct HeaderIconPresenter {}

mod ffi {
    use crate::{
        color::FfiColor,
        color_scheme::FfiColorScheme,
        transaction::{TransactionDirection, TransactionState},
    };

    use super::HeaderIconPresenter;

    #[uniffi::export]
    impl HeaderIconPresenter {
        #[uniffi::constructor]
        pub fn new() -> Self {
            Self {}
        }

        pub fn ring_color(
            &self,
            state: TransactionState,
            color_scheme: FfiColorScheme,
            direction: TransactionDirection,
            confirmations: i32,
            ring_number: i32,
        ) -> FfiColor {
            type S = TransactionState;
            type C = FfiColor;
            type X = FfiColorScheme;
            type D = TransactionDirection;

            match (state, direction, color_scheme, confirmations, ring_number) {
                (S::Pending, _, X::Dark, _, _) => C::White,
                (S::Pending, _, X::Light, _, _) => C::CoolGray,
                (S::Confirmed, D::Outgoing, X::Dark, _, _) => C::White,
                (S::Confirmed, D::Outgoing, X::Light, _, _) => C::Black,
                (S::Confirmed, D::Incoming, X::Dark, _, 3) => C::Green,
                (S::Confirmed, D::Incoming, X::Dark, confirmations, 2) => {
                    if confirmations > 1 {
                        C::Green
                    } else {
                        C::White
                    }
                }
                (S::Confirmed, D::Incoming, X::Dark, confirmations, 1) => {
                    if confirmations > 2 {
                        C::Green
                    } else {
                        C::White
                    }
                }
                (S::Confirmed, D::Incoming, X::Light, confirmations, 3) => {
                    if confirmations > 0 {
                        C::Green
                    } else {
                        C::Gray
                    }
                }
                (S::Confirmed, D::Incoming, X::Light, confirmations, 2) => {
                    if confirmations > 1 {
                        C::Green
                    } else {
                        C::Gray
                    }
                }
                (S::Confirmed, D::Incoming, X::Light, confirmations, 1) => {
                    if confirmations > 2 {
                        C::Green
                    } else {
                        C::Gray
                    }
                }
                (TransactionState::Confirmed, D::Incoming, _, _, _) => C::Green,
            }
        }
    }
}
