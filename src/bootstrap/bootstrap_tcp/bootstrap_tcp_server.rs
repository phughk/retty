use bytes::BytesMut;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use local_sync::mpsc::{
    unbounded::channel, unbounded::Rx as LocalReceiver, unbounded::Tx as LocalSender,
};
use log::{trace, warn};
use smol::{Async, Task, Timer};
use std::net::SocketAddr;
use std::{
    cell::RefCell,
    io::Error,
    net::{TcpListener, TcpStream},
    rc::Rc,
    time::{Duration, Instant},
};
use waitgroup::{WaitGroup, Worker};

use crate::bootstrap::{PipelineFactoryFn, MAX_DURATION_IN_SECS};
use crate::channel::InboundPipeline;
use crate::transport::{TaggedBytesMut, TransportContext};

/// A Bootstrap that makes it easy to bootstrap a pipeline to use for TCP servers.
pub struct BootstrapTcpServer<W> {
    pipeline_factory_fn: Option<Rc<PipelineFactoryFn<TaggedBytesMut, W>>>,
    close_tx: Rc<RefCell<Option<LocalSender<()>>>>,
    wg: Rc<RefCell<Option<WaitGroup>>>,
}

impl<W: 'static> Default for BootstrapTcpServer<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: 'static> BootstrapTcpServer<W> {
    /// Creates a new BootstrapTcpServer
    pub fn new() -> Self {
        Self {
            pipeline_factory_fn: None,
            close_tx: Rc::new(RefCell::new(None)),
            wg: Rc::new(RefCell::new(None)),
        }
    }

    /// Creates pipeline instances from when calling [BootstrapTcpServer::bind].
    pub fn pipeline(
        &mut self,
        pipeline_factory_fn: PipelineFactoryFn<TaggedBytesMut, W>,
    ) -> &mut Self {
        self.pipeline_factory_fn = Some(Rc::new(Box::new(pipeline_factory_fn)));
        self
    }

    /// Binds local address and port
    pub fn bind<A: ToString>(&self, addr: A) -> Result<SocketAddr, Error> {
        let listener = Async::<TcpListener>::bind(addr)?;
        let local_addr = listener.get_ref().local_addr()?;
        let pipeline_factory_fn = Rc::clone(self.pipeline_factory_fn.as_ref().unwrap());

        let (close_tx, mut close_rx) = channel();
        {
            let mut tx = self.close_tx.borrow_mut();
            *tx = Some(close_tx);
        }

        let worker = {
            let workgroup = WaitGroup::new();
            let worker = workgroup.worker();
            {
                let mut wg = self.wg.borrow_mut();
                *wg = Some(workgroup);
            }
            worker
        };

        Task::local(async move {
            let _w = worker;
            let child_wg = WaitGroup::new();

            //TODO
            let mut broadcast_close = vec![];

            loop {
                tokio::select! {
                    _ = close_rx.recv() => {
                        trace!("listener exit loop");
                        break;
                    }
                    res = listener.accept() => {
                        match res {
                            Ok((socket,_)) => {
                                // A new task is spawned for each inbound socket. The socket is
                                // moved to the new task and processed there.
                                let child_pipeline_factory_fn = Rc::clone(&pipeline_factory_fn);
                                let (child_close_tx, child_close_rx) = channel();
                                broadcast_close.push(child_close_tx);
                                let child_worker = child_wg.worker();
                                Task::local(async move {
                                    let _ = Self::process_pipeline(socket,
                                                                   child_pipeline_factory_fn,
                                                                   child_close_rx,
                                                                   child_worker).await;
                                }).detach();
                            }
                            Err(err) => {
                                warn!("listener accept error {}", err);
                                break;
                            }
                        }
                    }
                }
            }
            //TODO
            for child_close_tx in broadcast_close {
                let _ = child_close_tx.send(());
            }
            child_wg.wait().await;
        })
        .detach();

        Ok(local_addr)
    }

    async fn process_pipeline(
        mut socket: Async<TcpStream>,
        pipeline_factory_fn: Rc<PipelineFactoryFn<TaggedBytesMut, W>>,
        mut close_rx: LocalReceiver<()>,
        worker: Worker,
    ) -> Result<(), Error> {
        let _w = worker;

        let (sender, mut receiver) = channel();
        let pipeline = (pipeline_factory_fn)(sender);

        let local_addr = socket.get_ref().local_addr()?;
        let peer_addr = socket.get_ref().peer_addr()?;

        let mut buf = vec![0u8; 2048];

        pipeline.transport_active();
        loop {
            let mut eto = Instant::now() + Duration::from_secs(MAX_DURATION_IN_SECS);
            pipeline.poll_timeout(&mut eto);

            let delay_from_now = eto
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::from_secs(0));
            if delay_from_now.is_zero() {
                pipeline.handle_timeout(Instant::now());
                continue;
            }

            let timeout = Timer::after(delay_from_now);

            tokio::select! {
                _ = close_rx.recv() => {
                    trace!("pipeline socket exit loop");
                    break;
                }
                _ = timeout => {
                    pipeline.handle_timeout(Instant::now());
                }
                opt = receiver.recv() => {
                    if let Some(transmit) = opt {
                        match socket.write(&transmit.message).await {
                            Ok(n) => {
                                trace!("socket write {} bytes", n);
                            }
                            Err(err) => {
                                warn!("socket write error {}", err);
                                break;
                            }
                        }
                    } else {
                        warn!("pipeline recv error");
                        break;
                    }
                }
                res = socket.read(&mut buf) => {
                    match res {
                        Ok(n) => {
                            if n == 0 {
                                pipeline.read_eof();
                                break;
                            }

                            trace!("socket read {} bytes", n);
                            pipeline.read(TaggedBytesMut {
                                    now: Instant::now(),
                                    transport: TransportContext {
                                        local_addr,
                                        peer_addr,
                                        ecn: None,
                                    },
                                    message: BytesMut::from(&buf[..n]),
                                });
                        }
                        Err(err) => {
                            warn!("socket read error {}", err);
                            break;
                        }
                    }
                }
            }
        }
        pipeline.transport_inactive();

        Ok(())
    }

    /// Gracefully stop the server
    pub async fn stop(&self) {
        {
            let mut close_tx = self.close_tx.borrow_mut();
            if let Some(close_tx) = close_tx.take() {
                let _ = close_tx.send(());
            }
        }
        let wg = {
            let mut wg = self.wg.borrow_mut();
            wg.take()
        };
        if let Some(wg) = wg {
            wg.wait().await;
        }
    }
}
