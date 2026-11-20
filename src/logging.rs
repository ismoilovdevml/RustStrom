use log::{info, warn, LevelFilter};
use log4rs::{
    append::console::ConsoleAppender,
    config::{Appender, Logger, Root},
    encode::pattern,
    Config,
};
use pattern::PatternEncoder;

pub fn initialize() -> log4rs::Handle {
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
        .build(Root::builder().appender("stdout").build(level_filter))
        .unwrap();

    let handle = log4rs::init_config(config).expect("Initializing logging should not fail");
    if parsed_level.is_none() {
        warn!("Invalid log level '{}', falling back to INFO", log_level);
    }
    info!("Logging Level: {}", level_filter);
    handle
}

fn parse_level_filter(str: &str) -> Option<LevelFilter> {
    match str.to_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}
