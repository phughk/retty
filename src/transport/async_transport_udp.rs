use async_trait::async_trait;
use bytes::BytesMut;
use log::{trace, warn};
use std::io::ErrorKind;
use std::sync::Arc;

use crate::channel::handler::{
    Handler, InboundHandler, InboundHandlerContext, OutboundHandler, OutboundHandlerContext,
};
use crate::channel::handler_internal::{InboundHandlerInternal, OutboundHandlerInternal};
use crate::error::Error;
use crate::runtime::sync::Mutex;
use crate::transport::{AsyncTransportWrite, TransportContext};

struct AsyncTransportUdpDecoder;
struct AsyncTransportUdpEncoder {
    writer: Box<dyn AsyncTransportWrite + Send + Sync>,
}

pub struct AsyncTransportUdp {
    decoder: AsyncTransportUdpDecoder,
    encoder: AsyncTransportUdpEncoder,
}

impl AsyncTransportUdp {
    pub fn new(writer: Box<dyn AsyncTransportWrite + Send + Sync>) -> Self {
        AsyncTransportUdp {
            decoder: AsyncTransportUdpDecoder {},
            encoder: AsyncTransportUdpEncoder { writer },
        }
    }
}

#[derive(Default)]
pub struct TaggedBytesMut {
    pub transport: TransportContext,
    pub message: BytesMut,
}

#[async_trait]
impl InboundHandler for AsyncTransportUdpDecoder {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;

    async fn read(
        &mut self,
        ctx: &mut InboundHandlerContext<Self::Rin, Self::Rout>,
        msg: &mut Self::Rin,
    ) {
        ctx.fire_read(msg).await;
    }
}

#[async_trait]
impl OutboundHandler for AsyncTransportUdpEncoder {
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    async fn write(
        &mut self,
        ctx: &mut OutboundHandlerContext<Self::Win, Self::Wout>,
        msg: &mut Self::Win,
    ) {
        if let Some(target) = msg.transport.peer_addr {
            match self.writer.write(&msg.message, Some(target)).await {
                Ok(n) => {
                    trace!(
                        "AsyncTransportUdpEncoder --> write {} of {} bytes",
                        n,
                        msg.message.len()
                    );
                }
                Err(err) => {
                    warn!("AsyncTransportUdpEncoder write error: {}", err);
                    ctx.fire_write_exception(err.into()).await;
                }
            }
        } else {
            let err = Error::new(
                ErrorKind::NotConnected,
                String::from("Transport endpoint is not connected"),
            );
            ctx.fire_write_exception(err).await;
        }
    }
}

impl Handler for AsyncTransportUdp {
    type In = TaggedBytesMut;
    type Out = Self::In;

    fn name(&self) -> &str {
        "AsyncTransportUdp"
    }

    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandlerInternal>>,
        Arc<Mutex<dyn OutboundHandlerInternal>>,
    ) {
        let inbound_handler: Box<dyn InboundHandler<Rin = Self::In, Rout = Self::Out>> =
            Box::new(self.decoder);
        let outbound_handler: Box<dyn OutboundHandler<Win = Self::Out, Wout = Self::In>> =
            Box::new(self.encoder);

        (
            Arc::new(Mutex::new(inbound_handler)),
            Arc::new(Mutex::new(outbound_handler)),
        )
    }
}
