use async_trait::async_trait;
use log::{trace, warn};
use std::any::Any;
use std::io::ErrorKind;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Instant;

use crate::error::Error;
use crate::runtime::sync::Mutex;
use crate::transport::TransportContext;

#[async_trait]
pub trait InboundHandlerInternal: Send + Sync {
    async fn transport_active_internal(&mut self, ctx: &mut InboundHandlerContext);
    async fn transport_inactive_internal(&mut self, ctx: &mut InboundHandlerContext);

    async fn read_internal(
        &mut self,
        ctx: &mut InboundHandlerContext,
        message: &mut (dyn Any + Send + Sync),
    );
    async fn read_exception_internal(&mut self, ctx: &mut InboundHandlerContext, error: Error);
    async fn read_eof_internal(&mut self, ctx: &mut InboundHandlerContext);

    async fn read_timeout_internal(&mut self, ctx: &mut InboundHandlerContext, timeout: Instant);
    async fn poll_timeout_internal(
        &mut self,
        ctx: &mut InboundHandlerContext,
        timeout: &mut Instant,
    );
}

#[async_trait]
pub trait InboundHandler<T: Send + Sync + 'static>: Send + Sync {
    async fn transport_active(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_transport_active().await;
    }
    async fn transport_inactive(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_transport_inactive().await;
    }

    async fn read(&mut self, ctx: &mut InboundHandlerContext, message: &mut T) {
        ctx.fire_read(message).await;
    }
    async fn read_exception(&mut self, ctx: &mut InboundHandlerContext, error: Error) {
        ctx.fire_read_exception(error).await;
    }
    async fn read_eof(&mut self, ctx: &mut InboundHandlerContext) {
        ctx.fire_read_eof().await;
    }

    async fn read_timeout(&mut self, ctx: &mut InboundHandlerContext, timeout: Instant) {
        ctx.fire_read_timeout(timeout).await;
    }
    async fn poll_timeout(&mut self, ctx: &mut InboundHandlerContext, timeout: &mut Instant) {
        ctx.fire_poll_timeout(timeout).await;
    }
}

#[async_trait]
impl<T: Send + Sync + 'static> InboundHandlerInternal for Box<dyn InboundHandler<T>> {
    async fn transport_active_internal(&mut self, ctx: &mut InboundHandlerContext) {
        self.transport_active(ctx).await;
    }
    async fn transport_inactive_internal(&mut self, ctx: &mut InboundHandlerContext) {
        self.transport_inactive(ctx).await;
    }

    async fn read_internal(
        &mut self,
        ctx: &mut InboundHandlerContext,
        message: &mut (dyn Any + Send + Sync),
    ) {
        if let Some(msg) = message.downcast_mut::<T>() {
            self.read(ctx, msg).await;
        } else {
            ctx.fire_read_exception(Error::new(
                ErrorKind::Other,
                format!("message.downcast_mut error for {}", ctx.id),
            ))
            .await;
        }
    }
    async fn read_exception_internal(&mut self, ctx: &mut InboundHandlerContext, error: Error) {
        self.read_exception(ctx, error).await;
    }
    async fn read_eof_internal(&mut self, ctx: &mut InboundHandlerContext) {
        self.read_eof(ctx).await;
    }

    async fn read_timeout_internal(&mut self, ctx: &mut InboundHandlerContext, timeout: Instant) {
        self.read_timeout(ctx, timeout).await;
    }
    async fn poll_timeout_internal(
        &mut self,
        ctx: &mut InboundHandlerContext,
        timeout: &mut Instant,
    ) {
        self.poll_timeout(ctx, timeout).await;
    }
}

#[async_trait]
pub trait OutboundHandlerInternal: Send + Sync {
    async fn write_internal(
        &mut self,
        ctx: &mut OutboundHandlerContext,
        message: &mut (dyn Any + Send + Sync),
    );
    async fn write_exception_internal(&mut self, ctx: &mut OutboundHandlerContext, error: Error);
    async fn close_internal(&mut self, ctx: &mut OutboundHandlerContext);
}

#[async_trait]
pub trait OutboundHandler<T: Send + Sync + 'static>: Send + Sync {
    async fn write(&mut self, ctx: &mut OutboundHandlerContext, message: &mut T) {
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
impl<T: Send + Sync + 'static> OutboundHandlerInternal for Box<dyn OutboundHandler<T>> {
    async fn write_internal(
        &mut self,
        ctx: &mut OutboundHandlerContext,
        message: &mut (dyn Any + Send + Sync),
    ) {
        if let Some(msg) = message.downcast_mut::<T>() {
            self.write(ctx, msg).await;
        } else {
            ctx.fire_write_exception(Error::new(
                ErrorKind::Other,
                format!("message.downcast_mut error for {}", ctx.id),
            ))
            .await;
        }
    }
    async fn write_exception_internal(&mut self, ctx: &mut OutboundHandlerContext, error: Error) {
        self.write_exception(ctx, error).await;
    }
    async fn close_internal(&mut self, ctx: &mut OutboundHandlerContext) {
        self.close(ctx).await;
    }
}

pub trait Handler: Send + Sync {
    fn id(&self) -> String;

    #[allow(clippy::type_complexity)]
    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandlerInternal>>,
        Arc<Mutex<dyn OutboundHandlerInternal>>,
    );
}

#[derive(Default)]
pub struct InboundHandlerContext {
    pub(crate) id: String,

    pub(crate) next_in_ctx: Option<Arc<Mutex<InboundHandlerContext>>>,
    pub(crate) next_in_handler: Option<Arc<Mutex<dyn InboundHandlerInternal>>>,

    pub(crate) next_out: OutboundHandlerContext,
}

impl InboundHandlerContext {
    pub async fn fire_transport_active(&mut self) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.transport_active_internal(&mut next_ctx).await;
        }
    }

    pub async fn fire_transport_inactive(&mut self) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler
                .transport_inactive_internal(&mut next_ctx)
                .await;
        }
    }

    pub async fn fire_read(&mut self, message: &mut (dyn Any + Send + Sync)) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler.read_internal(&mut next_ctx, message).await;
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
            next_handler
                .read_exception_internal(&mut next_ctx, error)
                .await;
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
            next_handler.read_eof_internal(&mut next_ctx).await;
        } else {
            warn!("read_eof reached end of pipeline");
        }
    }

    pub async fn fire_read_timeout(&mut self, timeout: Instant) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler
                .read_timeout_internal(&mut next_ctx, timeout)
                .await;
        } else {
            warn!("read reached end of pipeline");
        }
    }

    pub async fn fire_poll_timeout(&mut self, timeout: &mut Instant) {
        if let (Some(next_in_handler), Some(next_in_ctx)) =
            (&self.next_in_handler, &self.next_in_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_in_handler.lock().await, next_in_ctx.lock().await);
            next_handler
                .poll_timeout_internal(&mut next_ctx, timeout)
                .await;
        } else {
            trace!("poll_timeout reached end of pipeline");
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
    pub(crate) id: String,
    pub(crate) transport_ctx: Option<TransportContext>,

    pub(crate) next_out_ctx: Option<Arc<Mutex<OutboundHandlerContext>>>,
    pub(crate) next_out_handler: Option<Arc<Mutex<dyn OutboundHandlerInternal>>>,
}

impl OutboundHandlerContext {
    pub fn get_transport(&self) -> TransportContext {
        *self.transport_ctx.as_ref().unwrap()
    }

    pub async fn fire_write(&mut self, message: &mut (dyn Any + Send + Sync)) {
        if let (Some(next_out_handler), Some(next_out_ctx)) =
            (&self.next_out_handler, &self.next_out_ctx)
        {
            let (mut next_handler, mut next_ctx) =
                (next_out_handler.lock().await, next_out_ctx.lock().await);
            next_handler.write_internal(&mut next_ctx, message).await;
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
            next_handler
                .write_exception_internal(&mut next_ctx, error)
                .await;
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
            next_handler.close_internal(&mut next_ctx).await;
        } else {
            warn!("close reached end of pipeline");
        }
    }
}
