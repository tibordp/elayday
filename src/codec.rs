use crate::error::ElaydayError;
use tokio_util::codec::{Decoder, Encoder};
use bytes::BytesMut;
use crate::elayday::Frame;
use prost::Message;

pub struct FrameCodec {}

impl FrameCodec {
    fn new() -> Self {
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