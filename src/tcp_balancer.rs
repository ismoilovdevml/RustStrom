use crate::config::{Backend, LoadBalancingAlgorithm};
use crate::algorithms::{round_robin, random_pick, ip_hash, least_connection, sticky_cookie};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::io::{self, Read, Write};
use log::{info, error, warn};
use hyper::Server as HyperServer;

pub fn start_tcp_load_balancer(address: &str, backends: Vec<Backend>, algorithm: LoadBalancingAlgorithm) {
    let listener = TcpListener::bind(address).unwrap_or_else(|_| panic!("Failed to bind to address: {}", address));
    info!("TCP Load Balancer started on {}", address);

    for stream in listener.incoming() {
        match stream {
            Ok(client) => {
                let backends_clone = backends.clone();
                thread::spawn(move || {
                    handle_client(client, &backends_clone, &algorithm).unwrap_or_else(|e| {
                        error!("Error handling client: {}", e);
                    });
                });
            },
            Err(e) => {
                error!("Error accepting client: {}", e);
            }
        }
    }
}

fn handle_client(mut client: TcpStream, backends: &[Backend], algorithm: &LoadBalancingAlgorithm) -> io::Result<()> {
    let backend = match algorithm {
        LoadBalancingAlgorithm::RoundRobin => round_robin(backends),
        LoadBalancingAlgorithm::Random => random_pick(backends),
        LoadBalancingAlgorithm::IPHash => ip_hash(client.peer_addr()?.ip(), backends),
        LoadBalancingAlgorithm::LeastConnection => least_connection(backends),
        LoadBalancingAlgorithm::StickyCookie => sticky_cookie(&client, backends),
    }.unwrap_or_else(|| {
        error!("No backend available for selected algorithm");
        &backends[0]  // Default to the first backend if no backend is selected
    });

    let mut server = TcpStream::connect((backend.ip.as_str(), backend.port)).map_err(|e| {
        error!("Error connecting to backend {}:{}", backend.ip, backend.port);
        e
    })?;

    let client_to_server = client.try_clone()?;
    let server_to_client = server.try_clone()?;

    thread::spawn(move || forward_data(client_to_server, &mut server));
    thread::spawn(move || forward_data(server_to_client, &mut client));

    Ok(())
}

fn forward_data(mut reader: TcpStream, writer: &mut TcpStream) {
    let mut buf = [0; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF, no more data to read
            Ok(n) => {
                if let Err(e) = writer.write_all(&buf[0..n]) {
                    warn!("Error writing data: {}", e);
                    break;
                }
            },
            Err(e) => {
                warn!("Error reading data: {}", e);
                break;
            }
        }
    }
}