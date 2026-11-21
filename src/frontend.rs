//! Connection handling in front of Pingora's HTTP proxy.
//!
//! Pingora 0.9 has no total deadline for a request head (every single read gets 60 s, so a
//! client trickling one header line at a time is never dropped) and a fixed 60 s TLS handshake
//! timeout. This [`ServerApp`] adds both:
//!
//! * TLS is terminated here (HTTPS listener) with a handshake deadline, the SNI certificate
//!   callback and ALPN `h2`/`http/1.1`;
//! * for HTTP/1.x every request head must arrive completely within the header timeout. The
//!   bytes read are then handed to Pingora as a pipelined prefix, so Pingora parses exactly what
//!   the client sent.
//!
//! HTTP/2 connections are passed to Pingora unchanged.

use crate::{proxy::Proxy, tls::SniResolver};
use async_trait::async_trait;
use bytes::BytesMut;
use log::{debug, warn};
use pingora_core::{
    apps::{HttpServerApp, ServerApp},
    protocols::{
        http::ServerSession,
        l4::stream::Stream as L4Stream,
        tls::{server::handshake_with_callback, ALPN},
        Stream,
    },
    server::ShutdownWatch,
    tls::ssl::{self, AlpnError, SslAcceptor, SslMethod},
};
use pingora_proxy::HttpProxy;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{timeout, timeout_at, Instant},
};

/// Maximum time between the first byte of a request head and its end (v1.1: 30 s).
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum time for a TLS handshake (v1.1: 10 s).
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long an idle keep-alive connection waits for the next request (Pingora's default).
pub const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(60);
/// Larger heads are handed to Pingora, which rejects them.
const MAX_HEAD: usize = 1024 * 1024;
/// ALPN wire format: h2 preferred, http/1.1 as fallback.
const ALPN_H2_H1: &[u8] = b"\x02h2\x08http/1.1";

pub struct Frontend {
    proxy: Arc<HttpProxy<Proxy>>,
    https_address: SocketAddr,
    tls: SslAcceptor,
    resolver: SniResolver,
}

enum Head {
    Complete(BytesMut),
    Closed,
    TimedOut,
}

impl Frontend {
    pub fn new(
        proxy: HttpProxy<Proxy>,
        https_address: SocketAddr,
        resolver: SniResolver,
    ) -> Result<Frontend, String> {
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())
            .map_err(|e| format!("Could not create TLS acceptor: {}", e))?;
        // Prefer h2, fall back to http/1.1; unknown protocols are ignored
        builder.set_alpn_select_callback(|_, client| {
            ssl::select_next_proto(ALPN_H2_H1, client).ok_or(AlpnError::NOACK)
        });
        Ok(Frontend {
            proxy: Arc::new(proxy),
            https_address,
            tls: builder.build(),
            resolver,
        })
    }

    fn is_https_listener(&self, stream: &Stream) -> bool {
        let local = stream
            .get_socket_digest()
            .and_then(|digest| digest.local_addr().and_then(|a| a.as_inet()).copied());
        match local {
            Some(local) if self.https_address.ip().is_unspecified() => {
                local.port() == self.https_address.port()
            }
            Some(local) => {
                local.port() == self.https_address.port()
                    && local.ip().to_canonical() == self.https_address.ip().to_canonical()
            }
            None => false,
        }
    }
}

/// Reads one request head (up to and including the empty line) with a total deadline.
async fn read_head(stream: &mut Stream, first: bool, shutdown: &ShutdownWatch) -> Head {
    let mut buf = BytesMut::with_capacity(4096);
    let mut shutdown = shutdown.clone();
    // Idle keep-alive connections wait for the first byte of the next request
    if !first {
        if *shutdown.borrow() {
            return Head::Closed;
        }
        let wait = timeout(KEEPALIVE_TIMEOUT, stream.read_buf(&mut buf));
        tokio::select! {
            read = wait => match read {
                Ok(Ok(n)) if n > 0 => {}
                _ => return Head::Closed,
            },
            _ = shutdown.changed() => return Head::Closed,
        }
    }
    let deadline = Instant::now() + HEADER_TIMEOUT;
    let mut scanned = 0;
    loop {
        if let Some(end) = head_end(&buf, scanned) {
            debug!("request head of {} bytes complete", end);
            return Head::Complete(buf);
        }
        if buf.len() > MAX_HEAD {
            return Head::Complete(buf);
        }
        scanned = buf.len().saturating_sub(3);
        match timeout_at(deadline, stream.read_buf(&mut buf)).await {
            Ok(Ok(0)) => {
                // EOF: an incomplete head is a broken request, nothing to answer
                return Head::Closed;
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return Head::Closed,
            Err(_) => return Head::TimedOut,
        }
    }
}

/// Position after the end of the head (`\r\n\r\n` or bare `\n\n`), searching from `from`.
fn head_end(buf: &[u8], from: usize) -> Option<usize> {
    let tail = buf.get(from..)?;
    tail.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| from + p + 4)
        .or_else(|| {
            tail.windows(2)
                .position(|w| w == b"\n\n")
                .map(|p| from + p + 2)
        })
}

#[async_trait]
impl ServerApp for Frontend {
    async fn process_new(
        self: &Arc<Self>,
        stream: Stream,
        shutdown: &ShutdownWatch,
    ) -> Option<Stream> {
        let mut stream = if self.is_https_listener(&stream) {
            // Listeners hand out plain TCP streams; the TLS layer needs the concrete type
            let tcp = match stream.into_any().downcast::<L4Stream>() {
                Ok(tcp) => *tcp,
                Err(_) => {
                    warn!("HTTPS listener produced an unexpected stream type");
                    return None;
                }
            };
            let handshake = handshake_with_callback(&self.tls, tcp, &self.resolver);
            match timeout(TLS_HANDSHAKE_TIMEOUT, handshake).await {
                Ok(Ok(tls)) => Box::new(tls) as Stream,
                Ok(Err(e)) => {
                    debug!("TLS handshake failed: {}", e);
                    return None;
                }
                Err(_) => {
                    debug!("TLS handshake timed out after {:?}", TLS_HANDSHAKE_TIMEOUT);
                    return None;
                }
            }
        } else {
            stream
        };

        if matches!(stream.selected_alpn_proto(), Some(ALPN::H2)) {
            // Pingora runs the HTTP/2 connection (streams, flow control, GOAWAY on shutdown)
            return self.proxy.process_new(stream, shutdown).await;
        }

        let mut first = true;
        let mut settings = None;
        loop {
            let head = match read_head(&mut stream, first, shutdown).await {
                Head::Complete(head) => head,
                Head::Closed => return None,
                Head::TimedOut => {
                    debug!("request head not complete within {:?}", HEADER_TIMEOUT);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                    let _ = stream.flush().await;
                    return None;
                }
            };
            let mut session = ServerSession::new_http1(stream);
            match settings.take() {
                Some(persistent) => pingora_core::apps::HttpPersistentSettings::apply_to_session(
                    persistent,
                    &mut session,
                ),
                None => session.set_keepalive(Some(KEEPALIVE_TIMEOUT.as_secs())),
            }
            if *shutdown.borrow() {
                session.set_keepalive(None);
            }
            session.set_pipelined_prefix(head);
            let reused = self.proxy.process_new_http(session, shutdown).await?;
            let (next, persistent) = reused.consume();
            stream = next;
            settings = persistent;
            first = false;
        }
    }

    async fn cleanup(&self) {
        self.proxy.http_cleanup().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_end_of_head() {
        assert_eq!(
            head_end(b"GET / HTTP/1.1\r\nHost: a\r\n\r\nbody", 0),
            Some(27)
        );
        assert_eq!(head_end(b"GET / HTTP/1.1\nHost: a\n\n", 0), Some(24));
        assert_eq!(head_end(b"GET / HTTP/1.1\r\nHost: a\r\n", 0), None);
        assert_eq!(head_end(b"abc", 10), None);
    }
}
