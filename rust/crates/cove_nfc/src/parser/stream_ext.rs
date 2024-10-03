use super::{stream, Stream};

pub trait ParserStreamExt<'a> {
    fn to_vec(&'a self) -> Vec<u8>;
    fn len(&'a self) -> usize;
    fn is_empty(&'a self) -> bool {
        self.len() == 0
    }
    fn to_stream(&'a self) -> Stream<'a>;
}

impl<'a> ParserStreamExt<'a> for Vec<u8> {
    fn to_vec(&self) -> Vec<u8> {
        <[u8]>::to_vec(self)
    }

    fn len(&self) -> usize {
        <[u8]>::len(self)
    }

    fn to_stream(&'a self) -> Stream<'a> {
        stream(self)
    }
}

impl<'a> ParserStreamExt<'a> for Stream<'a> {
    fn to_vec(&self) -> Vec<u8> {
        self.as_ref().to_vec()
    }

    fn len(&'a self) -> usize {
        self.as_ref().len()
    }

    fn to_stream(&self) -> Stream<'a> {
        *self
    }
}
