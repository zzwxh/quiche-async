#![forbid(unsafe_code)]

use std::{
    net::{SocketAddr, ToSocketAddrs},
    rc::Rc,
    time::Duration,
};

use quiche::{ConnectionId, RecvInfo};
use quiche_async::{Conn, H3Conn};
use ring::rand::{SecureRandom, SystemRandom};
use tokio::{net::UdpSocket, task::LocalSet};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    LocalSet::new().run_until(async_main()).await
}

async fn async_main() {
    let domain = "www.youtube.com";
    let peer = format!("{}:443", domain).to_socket_addrs().unwrap().next().unwrap();
    let bind = match peer {
        SocketAddr::V4(_) => "0.0.0.0:0",
        SocketAddr::V6(_) => "[::]:0",
    };
    let socket = UdpSocket::bind(bind).await.unwrap();
    let local = socket.local_addr().unwrap();

    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();
    config.load_verify_locations_from_directory("/etc/ssl/certs").unwrap();
    config.set_application_protos(quiche::h3::APPLICATION_PROTOCOL).unwrap();
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_stream_data_uni(1_000_000);

    let mut scid = [0; quiche::MAX_CONN_ID_LEN];
    SystemRandom::new().fill(&mut scid).unwrap();
    let scid = ConnectionId::from_ref(&scid);

    let quic_conn = Conn::connect(Some(domain), &scid, local, peer, &mut config).unwrap();

    let quic_conn = Rc::new(quic_conn);
    let socket = Rc::new(socket);

    tokio::task::spawn_local(read_loop(quic_conn.clone(), socket.clone(), local));
    tokio::task::spawn_local(write_loop(quic_conn.clone(), socket.clone()));

    quic_conn.is_established_async().await;

    println!("is_established");
    let config = quiche::h3::Config::new().unwrap();
    let mut h3_conn = H3Conn::with_transport(&quic_conn, &config).unwrap();
    let headers = &[
        quiche::h3::Header::new(b":method", b"GET"),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", domain.as_bytes()),
        quiche::h3::Header::new(b":path", b"/"),
        quiche::h3::Header::new(b"user-agent", b"quiche"),
    ];
    h3_conn.send_request(&quic_conn, headers, true).unwrap();
    let mut buf = [0; 65527];
    loop {
        match h3_conn.poll_async(&quic_conn).await {
            Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                println!("got response headers {:?} on stream id {}", list, stream_id);
            }

            Ok((stream_id, quiche::h3::Event::Data)) => {
                while let Ok(read) = h3_conn.recv_body(&quic_conn, stream_id, &mut buf) {
                    println!("got {} bytes of response data on stream {}", read, stream_id);

                    // print!("{}", String::from_utf8(buf[..read].to_vec()).unwrap());
                }
            }

            Ok((_stream_id, quiche::h3::Event::Finished)) => {
                println!("response received , closing...",);
            }

            Ok((_stream_id, quiche::h3::Event::Reset(e))) => {
                println!("request was reset by peer with {}, closing...", e);
            }

            Ok((_, quiche::h3::Event::PriorityUpdate)) => unreachable!(),

            Ok((goaway_id, quiche::h3::Event::GoAway)) => {
                println!("GOAWAY id={}", goaway_id);
            }

            Err(e) => {
                println!("HTTP/3 processing failed: {:?}", e);
            }
        }
    }
}

async fn read_loop(quic_conn: Rc<Conn>, socket: Rc<UdpSocket>, local: SocketAddr) {
    let mut buf = [0; 65527];
    loop {
        let timeout = quic_conn.timeout().unwrap_or(Duration::MAX);
        println!("timeout={:?}", timeout);
        let sleep = tokio::time::sleep(timeout);

        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                let (len, from) = result.unwrap();
                match quic_conn.recv(&mut buf[..len], RecvInfo { from, to: local }) {
                    Ok(len) => println!("processed {}", len),
                    Err(e) => println!("recv failed {}", e),
                }
            }
            _ = sleep => {
                println!("on timeout");
                quic_conn.on_timeout();
            }
        }
    }
}

async fn write_loop(quic_conn: Rc<Conn>, socket: Rc<UdpSocket>) {
    let mut buf = [0; 1200];
    loop {
        let (len, info) = quic_conn.send_async(&mut buf).await.unwrap();
        println!("written {}", len);
        socket.send_to(&buf[..len], info.to).await.unwrap();
    }
}
