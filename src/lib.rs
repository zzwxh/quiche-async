#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
    task::{Context, Poll, Waker},
};

use quiche::{h3, Config, Connection, ConnectionId, Error, RecvInfo, Result, SendInfo};

pub struct Conn {
    inner: RefCell<Connection>,
    send: Cell<Option<Waker>>,
    poll: Cell<Option<Waker>>,
    is_established: Cell<Option<Waker>>,
}

pub struct H3Conn {
    inner: h3::Connection,
}

impl Conn {
    pub fn connect(server_name: Option<&str>, scid: &ConnectionId, local: SocketAddr, peer: SocketAddr, config: &mut Config) -> Result<Self> {
        quiche::connect(server_name, scid, local, peer, config).map(|conn| Self {
            inner: RefCell::new(conn),
            send: Cell::new(None),
            poll: Cell::new(None),
            is_established: Cell::new(None),
        })
    }

    pub fn send(&self, out: &mut [u8]) -> Result<(usize, SendInfo)> {
        wake(&self.is_established);
        self.inner.borrow_mut().send(out)
    }

    pub fn recv(&self, buf: &mut [u8], info: RecvInfo) -> Result<usize> {
        wake(&self.send);
        wake(&self.poll);
        wake(&self.is_established);
        self.inner.borrow_mut().recv(buf, info)
    }

    pub fn on_timeout(&self) {
        wake(&self.send);
        self.inner.borrow_mut().on_timeout()
    }

    pub fn is_established(&self) -> bool {
        self.inner.borrow().is_established()
    }

    pub async fn send_async(&self, out: &mut [u8]) -> Result<(usize, SendInfo)> {
        std::future::poll_fn(|cx| match self.send(out) {
            Err(Error::Done) => set_waker(&self.send, cx),
            v => Poll::Ready(v),
        })
        .await
    }

    pub async fn is_established_async(&self) -> bool {
        std::future::poll_fn(|cx| match self.is_established() {
            false => set_waker(&self.is_established, cx),
            v => Poll::Ready(v),
        })
        .await
    }
}

impl H3Conn {
    pub fn with_transport(conn: &Conn, config: &h3::Config) -> h3::Result<Self> {
        wake(&conn.send);
        h3::Connection::with_transport(&mut conn.inner.borrow_mut(), config).map(|conn| Self { inner: conn })
    }

    pub fn send_request<T: h3::NameValue>(&mut self, conn: &Conn, headers: &[T], fin: bool) -> h3::Result<u64> {
        wake(&conn.send);
        self.inner.send_request(&mut conn.inner.borrow_mut(), headers, fin)
    }

    pub fn poll(&mut self, conn: &Conn) -> h3::Result<(u64, h3::Event)> {
        wake(&conn.send);
        self.inner.poll(&mut conn.inner.borrow_mut())
    }

    pub fn recv_body(&mut self, conn: &Conn, stream_id: u64, out: &mut [u8]) -> h3::Result<usize> {
        wake(&conn.send);
        self.inner.recv_body(&mut conn.inner.borrow_mut(), stream_id, out)
    }

    pub async fn poll_async(&mut self, conn: &Conn) -> h3::Result<(u64, h3::Event)> {
        std::future::poll_fn(|cx| match self.poll(conn) {
            Err(h3::Error::Done) => set_waker(&conn.poll, cx),
            v => Poll::Ready(v),
        })
        .await
    }
}

fn wake(waker: &Cell<Option<Waker>>) {
    if let Some(waker) = waker.take() {
        waker.wake();
    }
}

fn set_waker<T>(waker: &Cell<Option<Waker>>, cx: &Context<'_>) -> Poll<T> {
    waker.replace(Some(cx.waker().clone()));
    Poll::Pending
}
