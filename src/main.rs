mod config;
mod connection_handler;
mod algorithms;


fn main() {
    // Load the configuration from the TOML file
    let configuration = config::load_config();

    // Assuming that the frontend configuration will have a single key-value pair for simplicity
    // Extracting the bind address from the first frontend configuration
    if let Some((_, frontend_config)) = configuration.frontend.iter().next() {
        let bind_address = &frontend_config.bind;

        // Start the TCP listener and handle connections
        let listener = std::net::TcpListener::bind(bind_address)
            .expect("Failed to bind to address");

        println!("Load Balancer started on {}", bind_address);

        for stream in listener.incoming() {
            match stream {
                Ok(client) => {
                    std::thread::spawn(|| {
                        connection_handler::handle_client(client, &configuration);
                    });
                }
                Err(e) => {
                    eprintln!("Error accepting client: {}", e);
                }
            }
        }
    } else {
        eprintln!("No frontend configuration found in the config file.");
    }
}
