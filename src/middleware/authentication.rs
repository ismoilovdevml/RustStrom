//! HTTP Basic authentication (RFC 7617) against LDAP, as in v1.1, hardened against being used
//! as an LDAP amplifier:
//!
//! * successful logins are cached for 30 s, failed ones for 5 s, keyed by a salted SHA-256 of
//!   user and password (the password itself is never stored);
//! * at most `max_concurrent` LDAP exchanges run at once (default 32); above that the request
//!   is answered `503` immediately instead of queueing more LDAP connections.

use http::header::AUTHORIZATION;
use http_auth_basic::Credentials;
use ldap3::{ldap_escape, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry};
use log::{error, warn};
use openssl::sha::Sha256;
use pingora_http::RequestHeader;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const LDAP_TIMEOUT: Duration = Duration::from_secs(5);
/// Successful logins are trusted for this long; a changed or revoked password takes effect at
/// the latest after it.
const POSITIVE_TTL: Duration = Duration::from_secs(30);
/// Repeating the same wrong credentials does not reach LDAP again within this time.
const NEGATIVE_TTL: Duration = Duration::from_secs(5);
const CACHE_CAPACITY: usize = 10_000;
pub const DEFAULT_MAX_CONCURRENT: usize = 32;

/// Result of an authentication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    Allowed,
    Denied,
    /// Too many LDAP exchanges in flight; the request is rejected without asking LDAP.
    Busy,
}

type CacheKey = [u8; 32];

#[derive(Debug)]
pub struct Authentication {
    pub ldap_address: String,
    pub user_directory: String,
    pub rdn_identifier: String,
    pub recursive: bool,
    /// Value: when the result was stored and whether the login succeeded.
    cache: Mutex<HashMap<CacheKey, (Instant, bool)>>,
    salt: [u8; 32],
    ldap_slots: Semaphore,
}

impl Authentication {
    pub fn new(
        ldap_address: String,
        user_directory: String,
        rdn_identifier: String,
        recursive: bool,
        max_concurrent: usize,
    ) -> Authentication {
        Authentication {
            ldap_address,
            user_directory,
            rdn_identifier,
            recursive,
            cache: Mutex::new(HashMap::new()),
            salt: rand::random(),
            ldap_slots: Semaphore::new(max_concurrent.max(1)),
        }
    }

    pub async fn authenticate(&self, request: &RequestHeader) -> AuthOutcome {
        let Some(credentials) = credentials(request) else {
            return AuthOutcome::Denied;
        };
        let key = self.cache_key(&credentials);
        if let Some(allowed) = self.cached(&key) {
            return if allowed {
                AuthOutcome::Allowed
            } else {
                AuthOutcome::Denied
            };
        }
        let Ok(_slot) = self.ldap_slots.try_acquire() else {
            warn!("LDAP authentication saturated, rejecting request");
            return AuthOutcome::Busy;
        };
        let result = tokio::time::timeout(
            LDAP_TIMEOUT,
            self.check_user_credentials(&credentials.user_id, &credentials.password),
        )
        .await;
        match result {
            Ok(Ok(allowed)) => {
                self.remember(key, allowed);
                if allowed {
                    AuthOutcome::Allowed
                } else {
                    AuthOutcome::Denied
                }
            }
            // Infrastructure errors are not cached: they say nothing about the credentials
            Ok(Err(e)) => {
                error!("LDAP authentication failed: {}", e);
                AuthOutcome::Denied
            }
            Err(_) => {
                error!("LDAP authentication timed out after {:?}", LDAP_TIMEOUT);
                AuthOutcome::Denied
            }
        }
    }

    /// Salted SHA-256 of user and password (length-prefixed, so `a:bc` and `ab:c` differ).
    fn cache_key(&self, credentials: &Credentials) -> CacheKey {
        let mut hasher = Sha256::new();
        hasher.update(&self.salt);
        for part in [&credentials.user_id, &credentials.password] {
            hasher.update(&(part.len() as u64).to_be_bytes());
            hasher.update(part.as_bytes());
        }
        hasher.finish()
    }

    fn cached(&self, key: &CacheKey) -> Option<bool> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let (at, allowed) = cache.get(key)?;
        let ttl = if *allowed { POSITIVE_TTL } else { NEGATIVE_TTL };
        (at.elapsed() < ttl).then_some(*allowed)
    }

    fn remember(&self, key: CacheKey, allowed: bool) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_CAPACITY {
            cache.retain(|_, (at, allowed)| {
                at.elapsed() < if *allowed { POSITIVE_TTL } else { NEGATIVE_TTL }
            });
            if cache.len() >= CACHE_CAPACITY {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now(), allowed));
    }

    async fn check_user_credentials(&self, user: &str, password: &str) -> Result<bool, LdapError> {
        let settings = LdapConnSettings::new().set_conn_timeout(LDAP_TIMEOUT);
        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &self.ldap_address).await?;
        ldap3::drive!(conn);
        let scope = if self.recursive {
            Scope::Subtree
        } else {
            Scope::OneLevel
        };
        // Escape the user supplied value to prevent LDAP filter injection (RFC 4515)
        let filter = format!("({}={})", self.rdn_identifier, ldap_escape(user));
        let (result_entry, _) = ldap
            .search(&self.user_directory, scope, &filter, vec!["1.1"])
            .await?
            .success()?;

        // Exactly one entry must match, otherwise the identifier is ambiguous
        let authenticated = match result_entry.as_slice() {
            [entry] => {
                let dn = SearchEntry::construct(entry.clone()).dn;
                ldap.simple_bind(&dn, password).await?.success().is_ok()
            }
            _ => false,
        };
        let _ = ldap.unbind().await;
        Ok(authenticated)
    }
}

/// Basic credentials of the request. An empty user or password is rejected: an empty password
/// turns an LDAP simple bind into an unauthenticated bind, which most servers accept.
fn credentials(request: &RequestHeader) -> Option<Credentials> {
    let header = request.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let credentials = Credentials::from_header(header.to_string()).ok()?;
    if credentials.user_id.is_empty() || credentials.password.is_empty() {
        return None;
    }
    Some(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(max_concurrent: usize) -> Authentication {
        Authentication::new(
            "ldap://127.0.0.1:1".into(),
            "ou=users".into(),
            "uid".into(),
            true,
            max_concurrent,
        )
    }

    fn request(authorization: Option<&str>) -> RequestHeader {
        let mut req = RequestHeader::build("GET", b"/", None).unwrap();
        if let Some(value) = authorization {
            req.insert_header("Authorization", value).unwrap();
        }
        req
    }

    // "user:pw"
    const USER_PW: &str = "Basic dXNlcjpwdw==";

    #[test]
    fn rejects_missing_and_empty_credentials() {
        assert!(credentials(&request(None)).is_none());
        // "user:" -> empty password
        assert!(credentials(&request(Some("Basic dXNlcjo="))).is_none());
        assert!(credentials(&request(Some("Bearer xyz"))).is_none());
        assert!(credentials(&request(Some(USER_PW))).is_some());
    }

    #[tokio::test]
    async fn unreachable_ldap_denies() {
        assert_eq!(
            auth(32).authenticate(&request(Some(USER_PW))).await,
            AuthOutcome::Denied
        );
    }

    #[test]
    fn cache_keys_separate_credentials() {
        let first = auth(32);
        let good = credentials(&request(Some(USER_PW))).unwrap();
        // "user:px"
        let other = credentials(&request(Some("Basic dXNlcjpweA=="))).unwrap();
        first.remember(first.cache_key(&good), true);
        assert_eq!(first.cached(&first.cache_key(&good)), Some(true));
        assert_eq!(first.cached(&first.cache_key(&other)), None);
        first.remember(first.cache_key(&other), false);
        assert_eq!(first.cached(&first.cache_key(&other)), Some(false));
        // the same credentials hash differently under another salt
        let second = auth(32);
        assert_ne!(first.cache_key(&good), second.cache_key(&good));
    }

    #[tokio::test]
    async fn negative_cache_skips_ldap() {
        let auth = auth(32);
        let creds = credentials(&request(Some(USER_PW))).unwrap();
        auth.remember(auth.cache_key(&creds), false);
        // No LDAP slot is needed for a cached answer
        let _all = auth.ldap_slots.try_acquire_many(32).unwrap();
        assert_eq!(
            auth.authenticate(&request(Some(USER_PW))).await,
            AuthOutcome::Denied
        );
    }

    #[tokio::test]
    async fn saturated_ldap_fails_fast() {
        let auth = auth(1);
        let _busy = auth.ldap_slots.try_acquire().unwrap();
        let started = Instant::now();
        assert_eq!(
            auth.authenticate(&request(Some(USER_PW))).await,
            AuthOutcome::Busy
        );
        assert!(started.elapsed() < Duration::from_millis(100));
    }
}
