//! HTTP Basic authentication (RFC 7617) against LDAP, as in v1.1, plus a short-lived cache of
//! successful logins so that not every request opens an LDAP connection.

use http::header::AUTHORIZATION;
use http_auth_basic::Credentials;
use ldap3::{ldap_escape, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry};
use log::error;
use pingora_http::RequestHeader;
use std::{
    collections::{hash_map::RandomState, HashMap},
    hash::{BuildHasher, Hash, Hasher},
    sync::Mutex,
    time::{Duration, Instant},
};

const LDAP_TIMEOUT: Duration = Duration::from_secs(5);
/// Successful logins are trusted for this long; a changed or revoked password takes effect at
/// the latest after it.
const CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_CAPACITY: usize = 10_000;

#[derive(Debug)]
pub struct Authentication {
    pub ldap_address: String,
    pub user_directory: String,
    pub rdn_identifier: String,
    pub recursive: bool,
    /// Keyed by a randomly seeded hash of user and password; the password is never stored.
    cache: Mutex<HashMap<u64, Instant>>,
    hasher: RandomState,
}

impl Authentication {
    pub fn new(
        ldap_address: String,
        user_directory: String,
        rdn_identifier: String,
        recursive: bool,
    ) -> Authentication {
        Authentication {
            ldap_address,
            user_directory,
            rdn_identifier,
            recursive,
            cache: Mutex::new(HashMap::new()),
            hasher: RandomState::new(),
        }
    }

    /// Whether the request carries valid credentials.
    pub async fn authenticate(&self, request: &RequestHeader) -> bool {
        let Some(credentials) = credentials(request) else {
            return false;
        };
        let key = self.cache_key(&credentials);
        if self.cached(key) {
            return true;
        }
        let result = tokio::time::timeout(
            LDAP_TIMEOUT,
            self.check_user_credentials(&credentials.user_id, &credentials.password),
        )
        .await;
        match result {
            Ok(Ok(true)) => {
                self.remember(key);
                true
            }
            Ok(Ok(false)) => false,
            Ok(Err(e)) => {
                error!("LDAP authentication failed: {}", e);
                false
            }
            Err(_) => {
                error!("LDAP authentication timed out after {:?}", LDAP_TIMEOUT);
                false
            }
        }
    }

    fn cache_key(&self, credentials: &Credentials) -> u64 {
        let mut hasher = self.hasher.build_hasher();
        credentials.user_id.hash(&mut hasher);
        0u8.hash(&mut hasher);
        credentials.password.hash(&mut hasher);
        hasher.finish()
    }

    fn cached(&self, key: u64) -> bool {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&key).is_some_and(|at| at.elapsed() < CACHE_TTL)
    }

    fn remember(&self, key: u64) {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_CAPACITY {
            cache.retain(|_, at| at.elapsed() < CACHE_TTL);
            if cache.len() >= CACHE_CAPACITY {
                cache.clear();
            }
        }
        cache.insert(key, Instant::now());
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

    fn auth() -> Authentication {
        Authentication::new(
            "ldap://127.0.0.1:1".into(),
            "ou=users".into(),
            "uid".into(),
            true,
        )
    }

    fn request(authorization: Option<&str>) -> RequestHeader {
        let mut req = RequestHeader::build("GET", b"/", None).unwrap();
        if let Some(value) = authorization {
            req.insert_header("Authorization", value).unwrap();
        }
        req
    }

    #[test]
    fn rejects_missing_and_empty_credentials() {
        assert!(credentials(&request(None)).is_none());
        // "user:" -> empty password
        assert!(credentials(&request(Some("Basic dXNlcjo="))).is_none());
        assert!(credentials(&request(Some("Bearer xyz"))).is_none());
        // "user:pw"
        assert!(credentials(&request(Some("Basic dXNlcjpwdw=="))).is_some());
    }

    #[tokio::test]
    async fn unreachable_ldap_denies() {
        assert!(
            !auth()
                .authenticate(&request(Some("Basic dXNlcjpwdw==")))
                .await
        );
    }

    #[test]
    fn cache_only_remembers_exact_credentials() {
        let auth = auth();
        let good = credentials(&request(Some("Basic dXNlcjpwdw=="))).unwrap();
        // "user:px"
        let other = credentials(&request(Some("Basic dXNlcjpweA=="))).unwrap();
        auth.remember(auth.cache_key(&good));
        assert!(auth.cached(auth.cache_key(&good)));
        assert!(!auth.cached(auth.cache_key(&other)));
    }
}
