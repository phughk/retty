use async_trait::async_trait;
use log::warn;
use std::any::Any;
use std::io::ErrorKind;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::error::Error;
use crate::runtime::sync::Mutex;
use crate::transport::TransportContext;

pub type Message = dyn Any + Send + Sync;

#[async_trait]
pub trait InboundHandler: Send + Sync {
    async fn transport_active(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_transport_active().await;
    }
    async fn transport_inactive(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_transport_inactive().await;
    }
    async fn read(&mut self, ctx: &mut InboundHandlerContext, message: &mut Message) {
        ctx.fire_read(message).await;
    }
    async fn read_exception(&mut self, ctx: &mut InboundHandlerContext, error: Error) {
        ctx.fire_read_exception(error).await;
    }
    async fn read_eof(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_read_eof().await;
    }
}

#[async_trait]
pub trait InboundHandlerGeneric<T: Send + Sync + 'static>: Send + Sync {
    async fn read_generic(&mut self, ctx: &mut InboundHandlerContext, message: &mut T) {
        ctx.fire_read(message).await;
    }
}

#[async_trait]
impl<T: Send + Sync + 'static> InboundHandler for Box<dyn InboundHandlerGeneric<T>> {
    async fn read(&mut self, ctx: &mut InboundHandlerContext, message: &mut Message) {
        if let Some(msg) = message.downcast_mut::<T>() {
            self.read_generic(ctx, msg).await;
        } else {
            ctx.fire_read_exception(Error::new(
                ErrorKind::Other,
                String::from("message.downcast_mut error"),
            ))
            .await;
        }
    }
}

#[async_trait]
pub trait OutboundHandler: Send + Sync {
    async fn write(&mut self, ctx: &mut OutboundHandlerContext, message: &mut Message) {
        ctx.fire_write(message).await;
    }
    async fn write_exception(&mut self, ctx: &mut OutboundHandlerContext, error: Error) {
        ctx.fire_write_exception(error).await;
    }
    async fn close(&mut self, ctx: &mut OutboundHandlerContext) {
        ctx.fire_close().await;
    }
}

#[async_trait]
pub trait OutboundHandlerGeneric<T: Send + Sync + 'static>: Send + Sync {
    async fn write_generic(&mut self, ctx: &mut OutboundHandlerContext, message: &mut T) {
        ctx.fire_write(message).await;
    }
}

#[async_trait]
impl<T: Send + Sync + 'static> OutboundHandler for Box<dyn OutboundHandlerGeneric<T>> {
    async fn write(&mut self, ctx: &mut OutboundHandlerContext, message: &mut Message) {
        if let Some(msg) = message.downcast_mut::<T>() {
            self.write_generic(ctx, msg).await;
        } else {
            ctx.fire_write_exception(Error::new(
                ErrorKind::Other,
                String::from("message.downcast_mut error"),
            ))
            .await;
        }
    }
}

pub trait Handler: Send + Sync {
    fn id(&self) -> String;

    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandler>>,
        Arc<Mutex<dyn OutboundHandler>>,
    );
}

#[derive(Default)]
pub struct InboundHandlerContext {
    pub(crate) next_in_ctx: Option<Arc<Mutex<InboundHandlerContext>>>,
    pub(crate) next_in_handler: Option<Arc<Mutex<dyn InboundHandler>>>,

    pub(crate) next_out: OutboundHandlerContext,
}

impl InboundHandlerContext {
    pub async fn fire_transport_active(&mut self) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.transport_active(&mut next_ctx).await;
        }
    }

    pub async fn fire_transport_inactive(&mut self) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.transport_inactive(&mut next_ctx).await;
        }
    }

    pub async fn fire_read(&mut self, message: &mut Message) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.read(&mut next_ctx, message).await;
        } else {
            warn!("read reached end of pipeline");
        }
    }

    pub async fn fire_read_exception(&mut self, error: Error) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.read_exception(&mut next_ctx, error).await;
        } else {
            warn!("read_exception reached end of pipeline");
        }
    }

    pub async fn fire_read_eof(&mut self) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.read_eof(&mut next_ctx).await;
        } else {
            warn!("read_eof reached end of pipeline");
        }
    }
}

impl Deref for InboundHandlerContext {
    type Target = OutboundHandlerContext;
    fn deref(&self) -> &Self::Target {
        &self.next_out
    }
}

impl DerefMut for InboundHandlerContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.next_out
    }
}

#[derive(Default)]
pub struct OutboundHandlerContext {
    pub(crate) transport_ctx: Option<TransportContext>,

    pub(crate) next_out_ctx: Option<Arc<Mutex<OutboundHandlerContext>>>,
    pub(crate) next_out_handler: Option<Arc<Mutex<dyn OutboundHandler>>>,
}

impl OutboundHandlerContext {
    pub fn get_transport(&self) -> TransportContext {
        *self.transport_ctx.as_ref().unwrap()
    }

    pub async fn fire_write(&mut self, message: &mut Message) {
        if let (Some(next_out_handler), Some(next_out_ctx)) =
            (&self.next_out_handler, &self.next_out_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_out_handler.lock().await, next_out_ctx.lock().await);
            next_handler.write(&mut next_ctx, message).await;
        } else {
            warn!("write reached end of pipeline");
        }
    }

    pub async fn fire_write_exception(&mut self, error: Error) {
        if let (Some(next_out_handler), Some(next_out_ctx)) =
            (&self.next_out_handler, &self.next_out_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_out_handler.lock().await, next_out_ctx.lock().await);
            next_handler.write_exception(&mut next_ctx, error).await;
        } else {
            warn!("write_exception reached end of pipeline");
        }
    }

    pub async fn fire_close(&mut self) {
        if let (Some(next_out_handler), Some(next_out_ctx)) =
            (&self.next_out_handler, &self.next_out_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_out_handler.lock().await, next_out_ctx.lock().await);
            next_handler.close(&mut next_ctx).await;
        } else {
            warn!("close reached end of pipeline");
        }
    }
}
