use crate::elayday::Frame;
use crate::error::ElaydayError;
use bytes::BytesMut;
use prost::Message;
use tokio_util::codec::{Decoder, Encoder};

pub struct FrameCodec {}

impl FrameCodec {
    pub fn new() -> Self {
        Self {}
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = ElaydayError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        item.encode(dst)?;
        Ok(())
    }
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = ElaydayError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Frame::decode(src)?))
        }
    }
}
