use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
    task::{Poll, Waker},
};

use quiche::{Config, Connection, ConnectionId, Error, Result, SendInfo};

pub struct AsyncConn {
    inner: RefCell<Connection>,
    send: Cell<Option<Waker>>,
}

impl AsyncConn {
    pub async fn send(&self, out: &mut [u8]) -> Result<(usize, SendInfo)> {
        std::future::poll_fn(|cx| match self.inner.borrow_mut().send(out) {
            Err(Error::Done) => {
                self.send.replace(Some(cx.waker().clone()));
                Poll::Pending
            }
            v => Poll::Ready(v),
        })
        .await
    }
}

pub fn connect(server_name: Option<&str>, scid: &ConnectionId, local: SocketAddr, peer: SocketAddr, config: &mut Config) -> Result<AsyncConn> {
    quiche::connect(server_name, scid, local, peer, config).map(|conn| AsyncConn {
        inner: RefCell::new(conn),
        send: Cell::new(None),
    })
}
