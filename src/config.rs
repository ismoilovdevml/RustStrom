use std::fs;
use serde::Deserialize;
use serde_derive::Deserialize;
use serde::de::DeserializeOwned;

// Structs corresponding to the TOML configuration structure

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bind_address: String,
    pub backends: Backends,
    // Add placeholders for future configurations if needed
    // pub connection_timeout: Option<u64>,
    // pub health_check: Option<HealthCheck>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Backends {
    pub server: Vec<Backend>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Backend {
    pub ip: String,
    pub port: u16,
}

// Placeholder for future configurations
// #[derive(Debug, Deserialize)]
// pub struct HealthCheck {
//     pub interval: u64,
//     pub timeout: u64,
//     pub retries: u64,
// }

// Load the configuration from the TOML file
pub fn load_config() -> Config {
    let config_data = fs::read_to_string("configs/loadbalancer.toml")
        .expect("Failed to read configuration file");

    toml::from_str(&config_data)
        .expect("Failed to parse configuration file")
}

// Example usage (to be removed or modified as per the main function's requirements):
// let config = load_config();
// println!("{:?}", config);
