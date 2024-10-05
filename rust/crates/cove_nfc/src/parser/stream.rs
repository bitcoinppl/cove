use winnow::{Bytes, Partial};

pub type Stream<'i> = Partial<&'i Bytes>;
pub fn new(b: &[u8]) -> Stream<'_> {
    Partial::new(Bytes::new(b))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamOrVec<'a> {
    Stream(Stream<'a>),
    Vec(Vec<u8>),
}

impl<'a> StreamOrVec<'a> {
    pub fn len(&self) -> usize {
        match self {
            StreamOrVec::Stream(stream) => stream.len(),
            StreamOrVec::Vec(vec) => vec.len(),
        }
    }

    pub fn to_stream(&'a self) -> Stream<'a> {
        match self {
            StreamOrVec::Stream(stream) => *stream,
            StreamOrVec::Vec(vec) => new(vec),
        }
    }

    pub fn to_vec(self) -> Vec<u8> {
        match self {
            StreamOrVec::Stream(stream) => stream.to_vec(),
            StreamOrVec::Vec(vec) => vec,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a> From<Stream<'a>> for StreamOrVec<'a> {
    fn from(stream: Stream<'a>) -> Self {
        StreamOrVec::Stream(stream)
    }
}

impl From<Vec<u8>> for StreamOrVec<'_> {
    fn from(vec: Vec<u8>) -> Self {
        StreamOrVec::Vec(vec)
    }
}
