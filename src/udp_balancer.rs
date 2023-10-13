use super::config::Backend;
use std::net::{UdpSocket, SocketAddr};
use std::thread;
use std::str::FromStr;

pub fn start_udp_load_balancer(address: &str, backends: Vec<Backend>) {
    let socket = UdpSocket::bind(address).unwrap();
    let backends_addresses: Vec<SocketAddr> = backends.iter().map(|backend| {
        SocketAddr::from_str(&format!("{}:{}", backend.ip, backend.port)).unwrap()
    }).collect();

    let mut backend_index = 0;

    loop {
        let mut buf = [0; 4096];
        let (amt, src) = socket.recv_from(&mut buf).unwrap();

        // Basic round-robin mechanism to choose the backend
        let backend_address = &backends_addresses[backend_index];
        backend_index = (backend_index + 1) % backends_addresses.len();

        // Send data to the backend
        socket.send_to(&buf[0..amt], backend_address).unwrap();

        // Wait for the response and forward it back to the client
        let mut buf = [0; 4096];
        match socket.recv_from(&mut buf) {
            Ok((amt, _)) => {
                socket.send_to(&buf[0..amt], src).unwrap();
            }
            Err(e) => {
                eprintln!("Error receiving from backend: {}", e);
            }
        }
    }
}
