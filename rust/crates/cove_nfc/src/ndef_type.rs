#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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
