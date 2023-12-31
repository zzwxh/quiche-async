use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
    task::{Poll, Waker},
};

use quiche::{Config, Connection, ConnectionId, Error, RecvInfo, Result, SendInfo};

pub struct AsyncConn {
    inner: RefCell<Connection>,
    send_waker: Cell<Option<Waker>>,
}

impl AsyncConn {
    pub async fn send(&self, out: &mut [u8]) -> Result<(usize, SendInfo)> {
        std::future::poll_fn(|cx| match self.inner.borrow_mut().send(out) {
            Err(Error::Done) => {
                self.send_waker.replace(Some(cx.waker().clone()));
                Poll::Pending
            }
            v => Poll::Ready(v),
        })
        .await
    }

    pub fn recv(&self, buf: &mut [u8], info: RecvInfo) -> Result<usize> {
        self.wake_send();
        self.inner.borrow_mut().recv(buf, info)
    }

    pub fn on_timeout(&self) {
        self.wake_send();
        self.inner.borrow_mut().on_timeout()
    }

    fn wake_send(&self) {
        if let Some(waker) = self.send_waker.take() {
            waker.wake();
        }
    }
}

pub fn connect(server_name: Option<&str>, scid: &ConnectionId, local: SocketAddr, peer: SocketAddr, config: &mut Config) -> Result<AsyncConn> {
    quiche::connect(server_name, scid, local, peer, config).map(|conn| AsyncConn {
        inner: RefCell::new(conn),
        send_waker: Cell::new(None),
    })
}
