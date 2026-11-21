//! Integration tests against a real Pebble ACME CA.
//!
//! Start Pebble first (`tests/pebble.sh start`), then run
//! `cargo test -- --include-ignored`. Pebble validates every order with a real HTTP-01 request
//! to `PEBBLE_CHALLENGE_PORT`, where this file serves a shared [`ChallengeStore`].

use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rustls_pki_types::{pem::PemObject, CertificateDer};
use ruststrom_acme::{AcmeConfig, AcmeError, AcmeManager, CertPem, ChallengeStore};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

fn directory_url() -> String {
    env::var("PEBBLE_DIRECTORY_URL").unwrap_or_else(|_| "https://127.0.0.1:23400/dir".into())
}

fn pebble_root() -> PathBuf {
    env::var("PEBBLE_CA_ROOT")
        .unwrap_or_else(|_| "/data/ruststrom/acme/pebble/pebble.minica.pem".into())
        .into()
}

fn challenge_port() -> u16 {
    env::var("PEBBLE_CHALLENGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(23402)
}

/// A fresh name per test run, so Pebble never sees a name twice.
fn domain(tag: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{tag}-{nanos:x}-{n}.example.com")
}

fn config(dir: &Path) -> AcmeConfig {
    let mut cfg = AcmeConfig::new(directory_url(), "test@example.com", dir);
    cfg.ca_root_pem = Some(pebble_root());
    cfg.order_timeout = Duration::from_secs(60);
    cfg
}

fn cert_file(dir: &Path, domain: &str) -> PathBuf {
    let ca = directory_url()
        .replace("https://", "")
        .replace([':', '/'], "_");
    dir.join(ca).join("certs").join(format!("{domain}.pem"))
}

fn leaf_serial(cert: &CertPem) -> String {
    let der = CertificateDer::from_pem_slice(cert.chain_pem.as_bytes()).unwrap();
    let (_, parsed) = x509_parser::parse_x509_certificate(&der).unwrap();
    parsed.raw_serial_as_string()
}

// --- HTTP-01 responder -------------------------------------------------------------------------

struct Responder {
    store: ChallengeStore,
    /// (host, token path) pairs answered with a key authorization. Pebble fetches each token
    /// from several simulated vantage points, so one validation can appear more than once.
    validated: Mutex<HashSet<(String, String)>>,
}

/// Serves one shared `ChallengeStore` on the port Pebble's VA validates against. Runs on its
/// own thread and runtime so it outlives the per-test runtimes.
fn responder() -> &'static Arc<Responder> {
    static RESPONDER: OnceLock<Arc<Responder>> = OnceLock::new();
    RESPONDER.get_or_init(|| {
        let responder = Arc::new(Responder {
            store: ChallengeStore::default(),
            validated: Mutex::new(HashSet::new()),
        });
        let listener = std::net::TcpListener::bind(("127.0.0.1", challenge_port()))
            .expect("bind the HTTP-01 responder port");
        listener.set_nonblocking(true).unwrap();
        let r = responder.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = TcpListener::from_std(listener).unwrap();
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        continue;
                    };
                    tokio::spawn(answer(socket, r.clone()));
                }
            });
        });
        responder
    })
}

async fn answer(mut socket: TcpStream, responder: Arc<Responder>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let path = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let host = request
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("host").then(|| value.trim())
        })
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let response = match responder.store.response_for_path(path) {
        Some(body) => {
            responder
                .validated
                .lock()
                .unwrap()
                .insert((host, path.to_owned()));
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
        }
        None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
    };
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// Number of distinct HTTP-01 validations (tokens) the CA performed for `domain`.
fn was_validated(domain: &str) -> usize {
    responder()
        .validated
        .lock()
        .unwrap()
        .iter()
        .filter(|(host, _)| host == domain)
        .count()
}

/// A self-signed CA that did not sign Pebble's certificate.
fn unrelated_root(dir: &Path) -> PathBuf {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let path = dir.join("unrelated-root.pem");
    std::fs::write(&path, params.self_signed(&key).unwrap().pem()).unwrap();
    path
}

// --- tests -------------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn issues_certificate_with_real_http01_validation() {
    let dir = tempfile::tempdir().unwrap();
    let store = responder().store.clone();
    let manager = AcmeManager::new(config(dir.path()), store).unwrap();
    let name = domain("issue");

    let cert = manager.ensure_certificate(&name).await.unwrap();

    assert_eq!(was_validated(&name), 1, "exactly one HTTP-01 validation");
    assert!(
        cert.chain_pem.matches("BEGIN CERTIFICATE").count() >= 2,
        "leaf + intermediate"
    );
    assert!(cert.key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    let left = cert.not_after.duration_since(SystemTime::now()).unwrap();
    assert!(
        left > Duration::from_secs(80 * 24 * 3600),
        "Pebble issues 90 day certs"
    );

    // The issued key is ECDSA P-256 and the persisted bundle is private.
    let key = rcgen::KeyPair::from_pem(&cert.key_pem).unwrap();
    assert_eq!(key.algorithm(), &rcgen::PKCS_ECDSA_P256_SHA256);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ca_dir = cert_file(dir.path(), &name);
        let bundle = std::fs::metadata(&ca_dir).unwrap().permissions().mode();
        assert_eq!(bundle & 0o777, 0o600);
        let account = ca_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("account.json");
        let account = std::fs::metadata(account).unwrap().permissions().mode();
        assert_eq!(account & 0o777, 0o600);
    }
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn persisted_certificate_is_reused_without_contacting_the_ca() {
    let dir = tempfile::tempdir().unwrap();
    let name = domain("reuse");
    let first = AcmeManager::new(config(dir.path()), responder().store.clone())
        .unwrap()
        .ensure_certificate(&name)
        .await
        .unwrap();

    // Same manager config, but every connection to the CA fails TLS verification: the
    // certificate can only come from disk.
    let mut offline = config(dir.path());
    offline.ca_root_pem = Some(unrelated_root(dir.path()));
    let manager = AcmeManager::new(offline, ChallengeStore::default()).unwrap();
    let second = manager.ensure_certificate(&name).await.unwrap();
    let third = manager.ensure_certificate(&name).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(second, third);
    assert_eq!(was_validated(&name), 1, "no second validation");
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn account_is_persisted_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let a = AcmeManager::new(config(dir.path()), responder().store.clone()).unwrap();
    a.ensure_certificate(&domain("acct")).await.unwrap();
    let account_file = cert_file(dir.path(), "x")
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("account.json");
    let before = std::fs::read(&account_file).unwrap();

    let b = AcmeManager::new(config(dir.path()), responder().store.clone()).unwrap();
    b.ensure_certificate(&domain("acct")).await.unwrap();
    assert_eq!(before, std::fs::read(&account_file).unwrap());
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn renews_when_inside_the_renewal_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    // Pebble certificates live 90 days, so with a 365 day window every check is a renewal.
    cfg.renew_before = Duration::from_secs(365 * 24 * 3600);
    let manager = AcmeManager::new(cfg, responder().store.clone()).unwrap();
    let name = domain("renew");

    let first = manager.ensure_certificate(&name).await.unwrap();
    let second = manager.ensure_certificate(&name).await.unwrap();

    assert_ne!(leaf_serial(&first), leaf_serial(&second));
    assert_eq!(was_validated(&name), 2);
    // The renewed certificate replaced the old one on disk.
    let reloaded = AcmeManager::new(config(dir.path()), ChallengeStore::default())
        .unwrap()
        .ensure_certificate(&name)
        .await
        .unwrap();
    assert_eq!(reloaded, second);
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn renewal_loop_delivers_and_renews_only_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, CertPem)>();

    let mut due = config(dir.path());
    due.renew_before = Duration::from_secs(365 * 24 * 3600);
    due.check_interval = Duration::from_secs(1);
    let mut fresh = config(dir.path());
    fresh.check_interval = Duration::from_secs(1);

    let due_name = domain("loop-due");
    let fresh_name = domain("loop-fresh");
    let tx2 = tx.clone();
    let due_loop = tokio::spawn(
        Arc::new(AcmeManager::new(due, responder().store.clone()).unwrap()).run_renewal_loop(
            vec![due_name.clone()],
            move |d, c| {
                let _ = tx.send((d.to_owned(), c));
            },
        ),
    );
    let fresh_loop = tokio::spawn(
        Arc::new(AcmeManager::new(fresh, responder().store.clone()).unwrap()).run_renewal_loop(
            vec![fresh_name.clone(), "*.wildcard.example.com".into()],
            move |d, c| {
                let _ = tx2.send((d.to_owned(), c));
            },
        ),
    );

    let mut due_serials = Vec::new();
    let mut fresh_updates = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while due_serials.len() < 3 {
        let (d, cert) = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("renewal loop delivered too few updates")
            .unwrap();
        if d == due_name {
            due_serials.push(leaf_serial(&cert));
        } else {
            assert_eq!(d, fresh_name);
            fresh_updates += 1;
        }
    }
    // Let the fresh loop run a few more checks: it must not deliver again.
    tokio::time::sleep(Duration::from_secs(3)).await;
    while let Ok((d, _)) = rx.try_recv() {
        if d == fresh_name {
            fresh_updates += 1;
        }
    }
    due_loop.abort();
    fresh_loop.abort();

    due_serials.dedup();
    assert_eq!(
        due_serials.len(),
        3,
        "every due check delivers a new certificate"
    );
    assert_eq!(fresh_updates, 1, "a valid certificate is delivered once");
    assert_eq!(was_validated(&fresh_name), 1);
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn concurrent_orders_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let manager = AcmeManager::new(config(dir.path()), responder().store.clone()).unwrap();
    let (a, b) = (domain("conc-a"), domain("conc-b"));

    let (a1, b1, a2) = tokio::join!(
        manager.ensure_certificate(&a),
        manager.ensure_certificate(&b),
        manager.ensure_certificate(&a),
    );
    let (a1, b1, a2) = (a1.unwrap(), b1.unwrap(), a2.unwrap());

    assert_eq!(a1, a2, "same domain is ordered once");
    assert_ne!(a1.chain_pem, b1.chain_pem);
    assert_eq!(was_validated(&a), 1);
    assert_eq!(was_validated(&b), 1);
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn failed_validation_is_reported_and_cleaned_up() {
    let dir = tempfile::tempdir().unwrap();
    // Nobody serves this store, so Pebble's VA gets a 404.
    let unserved = ChallengeStore::default();
    let manager = AcmeManager::new(config(dir.path()), unserved.clone()).unwrap();
    let name = domain("fail");

    let err = manager.ensure_certificate(&name).await.unwrap_err();

    match &err {
        AcmeError::ValidationFailed { domain, detail } => {
            assert_eq!(domain, &name);
            assert!(
                detail.contains("404") || detail.contains("unauthorized"),
                "{detail}"
            );
        }
        other => panic!("expected ValidationFailed, got {other:?}"),
    }
    assert!(unserved.is_empty(), "challenge tokens must be removed");
    assert!(!cert_file(dir.path(), &name).exists());
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn ca_outage_keeps_the_existing_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let name = domain("outage");
    let issued = AcmeManager::new(config(dir.path()), responder().store.clone())
        .unwrap()
        .ensure_certificate(&name)
        .await
        .unwrap();

    // Due for renewal, but the CA cannot be reached: the still valid certificate is returned.
    let mut cfg = config(dir.path());
    cfg.renew_before = Duration::from_secs(365 * 24 * 3600);
    cfg.ca_root_pem = Some(unrelated_root(dir.path()));
    let manager = AcmeManager::new(cfg, ChallengeStore::default()).unwrap();
    assert_eq!(manager.ensure_certificate(&name).await.unwrap(), issued);

    // Without a certificate on disk the same outage is an error (the stored account cannot
    // even be loaded, because the directory fetch fails TLS verification).
    let err = manager
        .ensure_certificate(&domain("outage-new"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            AcmeError::Protocol {
                step: "loading the account",
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn pebble_is_not_trusted_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.ca_root_pem = None; // Mozilla roots only
    let err = AcmeManager::new(cfg, ChallengeStore::default())
        .unwrap()
        .ensure_certificate(&domain("untrusted"))
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("certificate"), "{message}");
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn rejected_identifier_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let manager = AcmeManager::new(config(dir.path()), responder().store.clone()).unwrap();
    let err = manager
        .ensure_certificate("blocked-domain.example")
        .await
        .unwrap_err();
    match err {
        AcmeError::Protocol { step, source } => {
            assert_eq!(step, "creating the order");
            assert!(
                source.to_string().contains("rejectedIdentifier"),
                "{source}"
            );
        }
        other => panic!("expected Protocol, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs Pebble: run tests/pebble.sh start"]
async fn order_timeout_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.order_timeout = Duration::from_millis(1);
    let manager = AcmeManager::new(cfg, responder().store.clone()).unwrap();
    let err = manager
        .ensure_certificate(&domain("timeout"))
        .await
        .unwrap_err();
    assert!(matches!(err, AcmeError::Timeout { .. }), "{err}");
}

#[tokio::test]
async fn invalid_domains_are_rejected_before_contacting_the_ca() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = AcmeConfig::new("https://127.0.0.1:23999/dir", "", dir.path());
    cfg.order_timeout = Duration::from_secs(5);
    let manager = AcmeManager::new(cfg, ChallengeStore::default()).unwrap();
    for bad in ["*.example.com", "10.0.0.1", "localhost", "a_b.example.com"] {
        let err = manager.ensure_certificate(bad).await.unwrap_err();
        assert!(
            matches!(err, AcmeError::InvalidDomain { .. }),
            "{bad}: {err}"
        );
    }
}
