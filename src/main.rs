mod config;
mod tcp_balancer;
mod udp_balancer;
mod utils;

fn main() {
    env_logger::init();

    // Load the configuration from the TOML file
    let configuration = config::load_config().expect("Failed to load configuration");

    // Extract the backends from the configuration
    let backends = configuration.backends.server;

    // Start the TCP load balancer with the loaded configuration
    tcp_balancer::start_tcp_load_balancer(&configuration.bind_address, backends);

    // For future implementations, you might also start the UDP load balancer here.
    // udp_balancer::start_udp_load_balancer(...);
}
