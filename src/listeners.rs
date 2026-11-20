use async_stream::stream;
use async_trait::async_trait;
use futures::Stream;
use log::{debug, error, info, warn};
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use tokio_rustls::server::TlsStream;
use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};

/// A client that does not finish the TLS handshake within this time is dropped.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Completed TLS handshakes waiting to be picked up by hyper.
const TLS_ACCEPT_QUEUE: usize = 1024;

pub struct HyperAcceptor<'a, T> {
    acceptor: Pin<Box<dyn Stream<Item = Result<T, io::Error>> + Send + 'a>>,
}

impl hyper::server::accept::Accept for HyperAcceptor<'_, TcpStream> {
    type Conn = TcpStream;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        Pin::new(&mut self.acceptor).poll_next(cx)
    }
}

impl hyper::server::accept::Accept for HyperAcceptor<'_, TlsStream<TcpStream>> {
    type Conn = TlsStream<TcpStream>;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        Pin::new(&mut self.acceptor).poll_next(cx)
    }
}

#[async_trait]
pub trait AcceptorProducer<T> {
    async fn produce_acceptor(
        self,
        address: SocketAddr,
    ) -> Result<HyperAcceptor<'async_trait, T>, io::Error>;
}

/// Accepts the next TCP connection, never giving up on transient errors.
///
/// Errors such as `EMFILE` (file descriptor limit reached) or `ECONNABORTED`
/// must not terminate the listener, otherwise the whole proxy goes down.
async fn accept_tcp(listener: &TcpListener) -> TcpStream {
    loop {
        match listener.accept().await {
            Ok((socket, _)) => {
                // Disable Nagle's algorithm for lower latency
                let _ = socket.set_nodelay(true);
                return socket;
            }
            Err(e) => {
                warn!("Failed to accept TCP connection: {}", e);
                // Back off briefly so a persistent error (e.g. EMFILE) does not spin the CPU
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

pub struct Http;

#[async_trait]
impl AcceptorProducer<TcpStream> for Http {
    async fn produce_acceptor(
        self,
        address: SocketAddr,
    ) -> Result<HyperAcceptor<'async_trait, TcpStream>, io::Error> {
        let listener = TcpListener::bind(address).await?;

        let incoming_stream = stream! {
          loop {
            yield Ok(accept_tcp(&listener).await);
          }
        };

        info!("Started listening for HTTP requests on {}", address);

        Ok(HyperAcceptor {
            acceptor: Box::pin(incoming_stream),
        })
    }
}

pub struct Https {
    pub tls_config: ServerConfig,
}

#[async_trait]
impl AcceptorProducer<TlsStream<TcpStream>> for Https {
    async fn produce_acceptor(
        self,
        address: SocketAddr,
    ) -> Result<HyperAcceptor<'async_trait, TlsStream<TcpStream>>, io::Error> {
        let tls_acceptor = TlsAcceptor::from(Arc::new(self.tls_config));
        let listener = TcpListener::bind(address).await?;
        let (sender, mut receiver) = mpsc::channel(TLS_ACCEPT_QUEUE);

        // Every handshake runs in its own task with a deadline, so a single slow
        // or malicious client can never block the accept loop.
        tokio::spawn(async move {
            loop {
                let socket = accept_tcp(&listener).await;
                if sender.is_closed() {
                    error!("HTTPS acceptor was dropped, stopping TLS accept loop");
                    break;
                }
                let tls_acceptor = tls_acceptor.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    match tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls_acceptor.accept(socket))
                        .await
                    {
                        Ok(Ok(tls_stream)) => {
                            let _ = sender.send(tls_stream).await;
                        }
                        Ok(Err(e)) => debug!("TLS handshake failed: {}", e),
                        Err(_) => debug!("TLS handshake timed out"),
                    }
                });
            }
        });

        let incoming_stream = stream! {
          while let Some(tls_stream) = receiver.recv().await {
            yield Ok(tls_stream);
          }
        };

        info!("Started listening for HTTPS requests on {}", address);

        Ok(HyperAcceptor {
            acceptor: Box::pin(incoming_stream),
        })
    }
}

pub trait RemoteAddress {
    fn remote_addr(&self) -> io::Result<SocketAddr>;
}

impl RemoteAddress for TcpStream {
    fn remote_addr(&self) -> io::Result<SocketAddr> {
        self.peer_addr()
    }
}

impl RemoteAddress for TlsStream<TcpStream> {
    fn remote_addr(&self) -> io::Result<SocketAddr> {
        let (stream, _) = self.get_ref();
        stream.peer_addr()
    }
}
