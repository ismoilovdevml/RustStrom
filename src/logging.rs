use log::{info, warn, LevelFilter};
use log4rs::{
    append::console::ConsoleAppender,
    config::{Appender, Logger, Root},
    encode::pattern::PatternEncoder,
    Config,
};

/// Initializes console logging. The level comes from `LOG_LEVEL` (or `RUST_LOG`), default INFO.
pub fn initialize() {
    let log_level = std::env::var("LOG_LEVEL")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "INFO".into());
    let parsed_level = parse_level_filter(&log_level);
    let level_filter = parsed_level.unwrap_or(LevelFilter::Info);

    let pattern = PatternEncoder::new("{d(%Y-%m-%d %H:%M:%S%.9f)} {({l}):5} {t} - {m}{n}");
    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(pattern))
        .build();
    let config = Config::builder()
        .logger(Logger::builder().build("ureq", LevelFilter::Warn))
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .build(Root::builder().appender("stdout").build(level_filter));
    let initialized = match config {
        Ok(config) => log4rs::init_config(config).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = initialized {
        eprintln!("Could not initialize logging: {}", e);
        return;
    }
    if parsed_level.is_none() {
        warn!("Invalid log level '{}', falling back to INFO", log_level);
    }
    info!("Logging Level: {}", level_filter);
}

fn parse_level_filter(level: &str) -> Option<LevelFilter> {
    match level.to_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}
