use super::config::Backend;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use log::{info, error, warn};

pub fn start_tcp_load_balancer(address: &str, backends: Vec<Backend>) {
    let listener = TcpListener::bind(address).unwrap_or_else(|_| panic!("Failed to bind to address: {}", address));
    info!("TCP Load Balancer started on {}", address);

    let backends_iter = backends.into_iter().cycle();
    let shared_backends = Arc::new(Mutex::new(backends_iter));

    for stream in listener.incoming() {
        match stream {
            Ok(client) => {
                let backends_clone = shared_backends.clone();
                thread::spawn(move || {
                    handle_client(client, backends_clone).unwrap_or_else(|e| {
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

fn handle_client(mut client: TcpStream, backends: Arc<Mutex<impl Iterator<Item = Backend>>>) -> io::Result<()> {
    let backend = backends.lock().unwrap().next().unwrap();

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
