use winnow::{Bytes, Partial};

pub type Stream<'i> = Partial<&'i Bytes>;
pub fn new(b: &[u8]) -> Stream<'_> {
    Partial::new(Bytes::new(b))
}

pub trait StreamExt {
    fn len(&self) -> usize;
    fn to_stream(&self) -> Stream;
    fn to_vec(self) -> Vec<u8>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StreamExt for Stream<'_> {
    fn len(&self) -> usize {
        self.as_ref().len()
    }

    fn to_stream(&self) -> Stream {
        *self
    }

    fn to_vec(self) -> Vec<u8> {
        self.as_ref().to_vec()
    }
}

impl StreamExt for Vec<u8> {
    fn len(&self) -> usize {
        self.len()
    }

    fn to_stream(&self) -> Stream {
        new(self)
    }

    fn to_vec(self) -> Vec<u8> {
        self
    }
}
