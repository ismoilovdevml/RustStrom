use std::net::TcpStream;
use std::io::{Read, Write, Error};
use super::config::{Config, Server};
use crate::algorithms;
use log::{error, warn, info};

pub fn handle_client(mut client: TcpStream, configuration: &Config) -> Result<(), Error> {
    // Extract domain from client request
    let domain_opt = extract_domain(&client);

    match domain_opt {
        Some(domain) => {
            if let Some(frontend_config) = configuration.frontend.get(&domain) {
                let backend_name = &frontend_config.default_backend;
                if let Some(backend_entry) = configuration.backend.get(backend_name) {
                    let server = algorithms::select_server(&backend_entry.servers, &backend_entry.balance);
                    // Forward the request to the selected server
                    forward_request_to_server(&mut client, &server)?;
                } else {
                    error!("Backend configuration not found for {}", backend_name);
                }
            } else {
                warn!("Frontend configuration not found for domain: {}", domain);
            }
        },
        None => error!("Domain extraction failed for client request"),
    }
    Ok(())
}

fn extract_domain(client: &TcpStream) -> Option<String> {
    let mut buffer = [0u8; 4096]; // Define buffer size

    // Peek into the stream to read data without consuming it
    if let Ok(bytes_read) = client.peek(&mut buffer) {
        if bytes_read == 0 {
            return None; // Connection was closed by client
        }

        // Convert buffer to string for parsing
        if let Ok(request_str) = std::str::from_utf8(&buffer[0..bytes_read]) {
            for line in request_str.lines() {
                if line.starts_with("Host:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() > 1 {
                        return Some(parts[1].to_string());
                    }
                }
            }
        }
    }
    None
}

fn forward_request_to_server(client: &mut TcpStream, server: &Server) -> Result<(), Error> {
    let mut buffer = [0u8; 4096];  // Define buffer size

    loop {
        let bytes_read = client.read(&mut buffer)?;

        if bytes_read == 0 {
            break; // Connection was closed by client
        }

        if let Some(backend_stream) = establish_backend_connection(server) {
            backend_stream.write_all(&buffer[0..bytes_read])?;
            // TODO: Continue forwarding data bidirectionally between client and server
            break; // Exit after successfully forwarding to one port
        }
    }
    Ok(())
}

fn establish_backend_connection(server: &Server) -> Option<TcpStream> {
    for port in &server.ports {
        let backend_address = format!("{}:{}", server.ip, port);
        if let Ok(backend_stream) = TcpStream::connect(&backend_address) {
            return Some(backend_stream);
        } else {
            warn!("Failed to connect to backend: {}", backend_address);
        }
    }
    None
}
