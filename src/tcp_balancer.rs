use super::config::Backend;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::io::{self, Read, Write};

pub fn start_tcp_load_balancer(address: &str, backends: Vec<Backend>) {
    let listener = TcpListener::bind(address).expect("Failed to bind to address");
    println!("TCP Load Balancer started on {}", address);

    for stream in listener.incoming() {
        match stream {
            Ok(client) => {
                let backends_clone = backends.clone();
                thread::spawn(|| {
                    if let Err(e) = handle_client(client, backends_clone) {
                        eprintln!("Error handling client: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Error accepting client: {}", e);
            }
        }
    }
}

fn handle_client(mut client: TcpStream, backends: Vec<Backend>) -> io::Result<()> {
    // Basic round-robin mechanism
    let backend = backends.iter().cycle().next().unwrap();

    // Connect to the backend
    let mut server = match TcpStream::connect((backend.ip.as_str(), backend.port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error connecting to backend {}:{}", backend.ip, backend.port);
            return Err(e);
        }
    };

    // Forward data between client and server using two threads
    let client_to_server = client.try_clone()?;
    let server_to_client = server.try_clone()?;

    thread::spawn(move || forward_data(client_to_server, server));
    thread::spawn(move || forward_data(server_to_client, client));

    Ok(())
}

fn forward_data(mut reader: TcpStream, mut writer: TcpStream) {
    let mut buf = [0; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF, no more data to read
                break;
            }
            Ok(n) => {
                if let Err(e) = writer.write_all(&buf[0..n]) {
                    eprintln!("Error writing data: {}", e);
                    break;
                }
            }
            Err(e) => {
                eprintln!("Error reading data: {}", e);
                break;
            }
        }
    }
}
