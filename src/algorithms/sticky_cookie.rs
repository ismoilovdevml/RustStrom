use super::super::config::Backend;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use crate::algorithms::round_robin;

const COOKIE_NAME: &str = "LB_BACKEND";

pub fn sticky_cookie(client: &TcpStream, backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    // Placeholder: Inspect the client's request for the LB_BACKEND cookie.
    // This requires parsing the HTTP headers from the raw TCP stream.
    let cookie_value = inspect_for_cookie(client, COOKIE_NAME);

    if let Some(value) = cookie_value {
        // If the cookie exists, map it to the corresponding backend.
        // This requires a mechanism to map cookie values to backends.
        map_cookie_to_backend(value, backends)
    } else {
        // If the cookie doesn't exist, pick a backend using another method and set the cookie.
        let backend = round_robin(backends);

        // Placeholder: Set the LB_BACKEND cookie in the response.
        // This requires modifying the HTTP headers in the raw TCP response.
        set_backend_cookie(client, backend.clone());

        backend
    }
}

fn inspect_for_cookie(_client: &TcpStream, _cookie_name: &str) -> Option<String> {
    // Placeholder: Parse the client's HTTP request for the specified cookie.
    // Return the cookie value if found.
    None
}

fn map_cookie_to_backend(_cookie_value: String, _backends: &Arc<Mutex<impl Iterator<Item = Backend>>>) -> Option<Backend> {
    // Placeholder: Map the cookie value to the corresponding backend.
    // This requires a mechanism to map cookie values to backends.
    None
}

fn set_backend_cookie(_client: &TcpStream, _backend: Option<Backend>) {
    // Placeholder: Set the specified cookie in the client's response.
    // This requires modifying the HTTP headers in the raw TCP response.
}
