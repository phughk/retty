use std::{future::Future, pin::Pin};

use super::Runtime;

/// A runtime for tokio
#[derive(Debug)]
pub struct TokioRuntime;

impl Runtime for TokioRuntime {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::spawn(future);
    }
}

pub mod sync {
    pub use tokio::sync::Mutex;
}

pub mod net {
    pub use tokio::net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream, ToSocketAddrs, UdpSocket,
    };
}

pub mod io {
    pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
}

pub mod mpsc {
    pub use tokio::sync::broadcast::{Receiver, Sender};
    pub fn bounded<T: Clone>(cap: usize) -> (Sender<T>, Receiver<T>) {
        tokio::sync::broadcast::channel(cap)
    }
}

pub use tokio::time::sleep;
