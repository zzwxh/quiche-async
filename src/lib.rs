use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
    task::{Poll, Waker},
};

use quiche::{h3, Config, Connection, ConnectionId, Error, RecvInfo, Result, SendInfo};

pub struct Conn {
    inner: RefCell<Connection>,
    send_waker: Cell<Option<Waker>>,
}

impl Conn {
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

pub fn connect(server_name: Option<&str>, scid: &ConnectionId, local: SocketAddr, peer: SocketAddr, config: &mut Config) -> Result<Conn> {
    quiche::connect(server_name, scid, local, peer, config).map(|conn| Conn {
        inner: RefCell::new(conn),
        send_waker: Cell::new(None),
    })
}

pub struct H3Conn {
    inner: h3::Connection,
}

impl H3Conn {
    pub fn with_transport(conn: &Conn, config: &h3::Config) -> h3::Result<H3Conn> {
        conn.wake_send();
        h3::Connection::with_transport(&mut conn.inner.borrow_mut(), config).map(|conn| H3Conn { inner: conn })
    }

    pub fn send_request<T: h3::NameValue>(&mut self, conn: &Conn, headers: &[T], fin: bool) -> h3::Result<u64> {
        conn.wake_send();
        self.inner.send_request(&mut conn.inner.borrow_mut(), headers, fin)
    }
}
