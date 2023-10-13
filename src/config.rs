use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Server {
    pub name: String,
    pub ip: String,
    pub ports: Vec<u16>,
}

#[derive(Debug, Deserialize)]
pub struct BackendEntry {
    pub domain: String,
    pub balance: String,
    pub servers: Vec<Server>,
}

#[derive(Debug, Deserialize)]
pub struct Frontend {
    pub bind: String,
}

#[derive(Debug, Deserialize)]
pub struct Backend {
    pub entries: Vec<BackendEntry>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub frontend: Frontend,
    pub backend: Backend,
}

pub fn load_config() -> Config {
    let config_data = std::fs::read_to_string("configs/loadbalancer.toml")
        .expect("Failed to read configuration file");
    toml::from_str(&config_data).expect("Failed to parse configuration file")
}