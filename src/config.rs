use std::fs;
use serde_derive::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bind_address: String,
    pub backends: Backends,
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

pub fn load_config() -> Result<Config, String> {
    let config_data = fs::read_to_string("configs/loadbalancer.toml")
        .map_err(|e| format!("Failed to read configuration file: {}", e))?;

    toml::from_str(&config_data)
        .map_err(|e| format!("Failed to parse configuration file: {}", e))
}