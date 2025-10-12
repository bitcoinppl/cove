use std::sync::Arc;

use cove_types::amount::Amount;

#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum AmountOrMax {
    Amt(Arc<Amount>),
    Max,
}
