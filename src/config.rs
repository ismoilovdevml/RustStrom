use serde_derive::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub frontend: std::collections::HashMap<String, Frontend>,
    pub backend: std::collections::HashMap<String, Backend>,
}

#[derive(Debug, Deserialize)]
pub struct Frontend {
    pub bind: String,
    pub mode: String,
    pub default_backend: String,
}

#[derive(Debug, Deserialize)]
pub struct Backend {
    pub mode: String,
    pub balance: String,
    pub servers: Vec<Server>,
}

#[derive(Debug, Deserialize)]
pub struct Server {
    pub name: String,
    pub ip: String,
    pub ports: Vec<u16>,
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_data = std::fs::read_to_string("configs/loadbalancer.toml")?;
    Ok(toml::from_str(&config_data)?)
}