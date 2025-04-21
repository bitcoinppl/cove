#[derive(Debug, Copy, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NdefType {
    Empty,
    WellKnown,
    Mime,
    AbsoluteUri,
    External,
    Unknown,
    Unchanged,
    Reserved,
}