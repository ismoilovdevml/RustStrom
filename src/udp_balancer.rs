use super::config::Backend;
use std::net::{UdpSocket, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn start_udp_load_balancer(address: &str, backends: Vec<Backend>) -> Result<(), std::io::Error> {
    let socket = UdpSocket::bind(address)?;
    println!("UDP Load Balancer started on {}", address);

    let backend_index = Arc::new(AtomicUsize::new(0));

    let mut buf = [0; 4096];
    loop {
        let (amt, src) = socket.recv_from(&mut buf)?;

        // Basic round-robin mechanism to select backend
        let index = backend_index.fetch_add(1, Ordering::Relaxed) % backends.len();
        let backend = &backends[index];

        let backend_address = format!("{}:{}", backend.ip, backend.port).parse::<SocketAddr>().unwrap();

        // Forward the packet to the selected backend
        socket.send_to(&buf[0..amt], &backend_address)?;
    }
}
