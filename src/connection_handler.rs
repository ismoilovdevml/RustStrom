use std::net::TcpStream;
use std::io::{Read, Write};
use super::config::{Config, Server};
use crate::algorithms;

pub fn handle_client(mut client: std::net::TcpStream, configuration: &Config) {
    // Extract domain from client request
    let domain_opt = extract_domain(&client);

    if let Some(domain) = domain_opt {
        if let Some(frontend_config) = configuration.frontend.get(&domain) {
            let backend_name = &frontend_config.default_backend;
            if let Some(backend_entry) = configuration.backend.get(backend_name) {
                
                let server = algorithms::select_server(&backend_entry.servers, &backend_entry.balance);

                // Forward the request to the selected server
                forward_request_to_server(&mut client, &server);
            } else {
                // Log: Backend configuration not found
            }
        } else {
            // Log: Frontend configuration not found for domain
        }
    } else {
        // Log: Domain extraction failed
    }
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

fn forward_request_to_server(client: &mut TcpStream, server: &Server) {
    let mut buffer = [0u8; 4096];  // Define buffer size

    let bytes_read = client.read(&mut buffer).unwrap_or(0);
    
    if bytes_read == 0 {
        return; // Connection was closed by client
    }

    for port in &server.ports {
        let backend_address = format!("{}:{}", server.ip, port);
        if let Ok(mut backend_stream) = TcpStream::connect(&backend_address) {
            backend_stream.write_all(&buffer[0..bytes_read]).unwrap_or_default();
            // TODO: Continue forwarding data bidirectionally between client and server
            break; // Exit after successfully forwarding to one port
        }
    }
}
