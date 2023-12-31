use std::{
    net::{SocketAddr, ToSocketAddrs},
    rc::Rc,
};

use quiche::{ConnectionId, RecvInfo};
use quiche_async::{Conn, H3Conn};
use ring::rand::{SecureRandom, SystemRandom};
use tokio::{net::UdpSocket, runtime::Builder, task::LocalSet};

fn main() {
    let rt = Builder::new_current_thread().enable_io().build().unwrap();
    let set = LocalSet::new();
    set.block_on(&rt, async_main());
}

async fn async_main() {
    let peer = "quic.aiortc.org:443".to_socket_addrs().unwrap().next().unwrap();
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

    let quic_conn = quiche_async::connect(Some("quic.aiortc.org"), &scid, local, peer, &mut config).unwrap();

    let quic_conn = Rc::new(quic_conn);
    let socket = Rc::new(socket);

    tokio::task::spawn_local(read_loop(quic_conn.clone(), socket.clone(), local));
    tokio::task::spawn_local(write_loop(quic_conn.clone(), socket.clone()));

    quic_conn.wait_for_established().await;

    println!("is_established");
    let config = quiche::h3::Config::new().unwrap();
    let mut h3_conn = H3Conn::with_transport(&quic_conn, &config).unwrap();
    let headers = &[
        quiche::h3::Header::new(b":method", b"GET"),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", b"quic.aiortc.org"),
        quiche::h3::Header::new(b":path", b"/"),
        quiche::h3::Header::new(b"user-agent", b"quiche"),
    ];
    h3_conn.send_request(&quic_conn, headers, true).unwrap();
    let mut buf = [0; 65527];
    loop {
        match h3_conn.poll(&quic_conn).await {
            Ok((stream_id, quiche::h3::Event::Headers { list, .. })) => {
                println!("got response headers {:?} on stream id {}", list, stream_id);
            }

            Ok((stream_id, quiche::h3::Event::Data)) => {
                while let Ok(read) = h3_conn.recv_body(&quic_conn, stream_id, &mut buf) {
                    println!("got {} bytes of response data on stream {}", read, stream_id);

                    print!("{}", String::from_utf8(buf[..read].to_vec()).unwrap());
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
        let (len, from) = socket.recv_from(&mut buf).await.unwrap();
        println!("got {}", len);
        match quic_conn.process_incoming_packet(&mut buf[..len], RecvInfo { from, to: local }) {
            Ok(len) => println!("processed {}", len),
            Err(e) => println!("recv failed {}", e),
        }
    }
}

async fn write_loop(quic_conn: Rc<Conn>, socket: Rc<UdpSocket>) {
    let mut buf = [0; 1200];
    loop {
        let (len, info) = quic_conn.generate_outgoing_packet(&mut buf).await.unwrap();
        socket.send_to(&buf[..len], info.to).await.unwrap();
        println!("written {}", len);
    }
}
