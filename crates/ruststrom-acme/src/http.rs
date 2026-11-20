//! HTTPS client used to talk to the ACME CA.
//!
//! instant-acme's built-in client pulls in the platform verifier and relies on rustls' process
//! wide default crypto provider (which panics when both ring and aws-lc-rs end up enabled in
//! one binary). This client pins the ring provider explicitly, trusts either the bundled
//! Mozilla roots or exactly one custom root, and bounds every request with a timeout.

use std::{future::Future, path::Path, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use http::Request;
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use instant_acme::{BodyWrapper, BytesResponse, HttpClient};
use rustls::RootCertStore;
use rustls_pki_types::{pem::PemObject, CertificateDer};

use crate::AcmeError;

/// Upper bound for a single HTTP request to the CA.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type HyperClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>;

#[derive(Clone)]
pub(crate) struct AcmeHttpClient {
    client: HyperClient,
}

impl AcmeHttpClient {
    /// Builds a client that trusts the Mozilla root store, or only the certificates in
    /// `custom_root_pem` when given.
    pub(crate) fn new(custom_root_pem: Option<&Path>) -> Result<Self, AcmeError> {
        let mut roots = RootCertStore::empty();
        match custom_root_pem {
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            Some(path) => {
                let pem = std::fs::read(path)
                    .map_err(|e| AcmeError::io("failed to read CA root", path, e))?;
                for cert in CertificateDer::pem_slice_iter(&pem) {
                    let cert = cert.map_err(|e| {
                        AcmeError::Config(format!("invalid PEM in {}: {e}", path.display()))
                    })?;
                    roots.add(cert).map_err(|e| {
                        AcmeError::Config(format!("invalid CA root in {}: {e}", path.display()))
                    })?;
                }
                if roots.is_empty() {
                    return Err(AcmeError::Config(format!(
                        "no certificate found in CA root file {}",
                        path.display()
                    )));
                }
            }
        }

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| AcmeError::Config(format!("TLS setup failed: {e}")))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_only()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Ok(Self { client })
    }
}

/// "client error (Connect): tcp connect error: Connection refused" instead of only the outer
/// message, which for hyper is rarely enough to act on.
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

impl HttpClient for AcmeHttpClient {
    fn request(
        &self,
        req: Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
        let uri = req.uri().to_string();
        let fut = self.client.request(req);
        Box::pin(async move {
            match tokio::time::timeout(REQUEST_TIMEOUT, fut).await {
                Ok(Ok(rsp)) => Ok(BytesResponse::from(rsp)),
                Ok(Err(e)) => Err(instant_acme::Error::Other(
                    format!("request to {uri} failed: {}", error_chain(&e)).into(),
                )),
                Err(_) => Err(instant_acme::Error::Other(
                    format!("request to {uri} timed out after {REQUEST_TIMEOUT:?}").into(),
                )),
            }
        })
    }
}
