use std::{
    cell::{Cell, RefCell},
    net::SocketAddr,
    task::{Poll, Waker},
};

use quiche::{h3, Config, Connection, ConnectionId, Error, RecvInfo, Result, SendInfo};

pub struct Conn {
    inner: RefCell<Connection>,
    send: Cell<Option<Waker>>,
    is_established: Cell<Option<Waker>>,
    poll: Cell<Option<Waker>>,
}

impl Conn {
    pub async fn generate_outgoing_packet(&self, out: &mut [u8]) -> Result<(usize, SendInfo)> {
        std::future::poll_fn(|cx| {
            wake(&self.is_established);
            match self.inner.borrow_mut().send(out) {
                Err(Error::Done) => {
                    self.send.replace(Some(cx.waker().clone()));
                    Poll::Pending
                }
                v => Poll::Ready(v),
            }
        })
        .await
    }

    pub fn process_incoming_packet(&self, buf: &mut [u8], info: RecvInfo) -> Result<usize> {
        wake(&self.send);
        wake(&self.is_established);
        wake(&self.poll);
        self.inner.borrow_mut().recv(buf, info)
    }

    pub fn on_timeout(&self) {
        wake(&self.send);
        self.inner.borrow_mut().on_timeout()
    }

    pub async fn wait_for_established(&self) {
        std::future::poll_fn(|cx| match self.inner.borrow().is_established() {
            false => {
                self.is_established.replace(Some(cx.waker().clone()));
                Poll::Pending
            }
            true => Poll::Ready(()),
        })
        .await
    }
}

fn wake(waker: &Cell<Option<Waker>>) {
    if let Some(waker) = waker.take() {
        waker.wake();
    }
}

pub fn connect(server_name: Option<&str>, scid: &ConnectionId, local: SocketAddr, peer: SocketAddr, config: &mut Config) -> Result<Conn> {
    quiche::connect(server_name, scid, local, peer, config).map(|conn| Conn {
        inner: RefCell::new(conn),
        send: Cell::new(None),
        is_established: Cell::new(None),
        poll: Cell::new(None),
    })
}

pub struct H3Conn {
    inner: h3::Connection,
}

impl H3Conn {
    pub fn with_transport(conn: &Conn, config: &h3::Config) -> h3::Result<H3Conn> {
        wake(&conn.send);
        h3::Connection::with_transport(&mut conn.inner.borrow_mut(), config).map(|conn| H3Conn { inner: conn })
    }

    pub fn send_request<T: h3::NameValue>(&mut self, conn: &Conn, headers: &[T], fin: bool) -> h3::Result<u64> {
        wake(&conn.send);
        self.inner.send_request(&mut conn.inner.borrow_mut(), headers, fin)
    }

    pub async fn poll(&mut self, conn: &Conn) -> h3::Result<(u64, h3::Event)> {
        std::future::poll_fn(|cx| {
            wake(&conn.send);
            match self.inner.poll(&mut conn.inner.borrow_mut()) {
                Err(h3::Error::Done) => {
                    conn.poll.replace(Some(cx.waker().clone()));
                    Poll::Pending
                }
                v => Poll::Ready(v),
            }
        })
        .await
    }

    pub fn recv_body(&mut self, conn: &Conn, stream_id: u64, out: &mut [u8]) -> h3::Result<usize> {
        wake(&conn.send);
        self.inner.recv_body(&mut conn.inner.borrow_mut(), stream_id, out)
    }
}
