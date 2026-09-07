use winnow::{Bytes, Partial};

pub type Stream<'i> = Partial<&'i Bytes>;

#[must_use]
pub fn new(b: &[u8]) -> Stream<'_> {
    Partial::new(Bytes::new(b))
}
