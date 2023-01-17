use async_trait::async_trait;
use bytes::BytesMut;
use std::sync::Arc;

use crate::channel::handler::*;
use crate::error::Error;
use crate::runtime::sync::Mutex;

pub mod line_based_frame_decoder;
pub mod tagged;

pub trait MessageDecoder {
    fn id(&self) -> String;
    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BytesMut>, Error>;
}

struct ByteToMessageDecoder {
    transport_active: bool,
    message_decoder: Box<dyn MessageDecoder + Send + Sync>,
}

struct ByteToMessageEncoder;

pub struct ByteToMessageCodec {
    decoder: ByteToMessageDecoder,
    encoder: ByteToMessageEncoder,
}

impl ByteToMessageCodec {
    pub fn new(message_decoder: Box<dyn MessageDecoder + Send + Sync>) -> Self {
        Self {
            decoder: ByteToMessageDecoder {
                transport_active: false,
                message_decoder,
            },
            encoder: ByteToMessageEncoder {},
        }
    }
}

#[async_trait]
impl InboundHandler<BytesMut> for ByteToMessageDecoder {
    async fn transport_active(&mut self, ctx: &mut InboundHandlerContext) {
        self.transport_active = true;
        ctx.fire_transport_active().await;
    }
    async fn transport_inactive(&mut self, ctx: &mut InboundHandlerContext) {
        self.transport_active = false;
        ctx.fire_transport_inactive().await;
    }
    async fn read(&mut self, ctx: &mut InboundHandlerContext, message: &mut BytesMut) {
        while self.transport_active {
            match self.message_decoder.decode(message) {
                Ok(msg) => {
                    if let Some(mut msg) = msg {
                        ctx.fire_read(&mut msg).await;
                    } else {
                        return;
                    }
                }
                Err(err) => {
                    ctx.fire_read_exception(err).await;
                    return;
                }
            }
        }
    }
}

impl OutboundHandler<BytesMut> for ByteToMessageEncoder {}

impl Handler for ByteToMessageCodec {
    fn id(&self) -> String {
        self.decoder.message_decoder.id()
    }

    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandlerInternal>>,
        Arc<Mutex<dyn OutboundHandlerInternal>>,
    ) {
        let decoder: Box<dyn InboundHandler<BytesMut>> = Box::new(self.decoder);
        let encoder: Box<dyn OutboundHandler<BytesMut>> = Box::new(self.encoder);
        (Arc::new(Mutex::new(decoder)), Arc::new(Mutex::new(encoder)))
    }
}
