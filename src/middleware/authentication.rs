use super::{Context, Middleware};
use async_trait::async_trait;
use http_auth_basic::Credentials;
use hyper::{
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    Body, HeaderMap, Request, Response, StatusCode,
};
use ldap3::{ldap_escape, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry};
use log::error;
use std::time::Duration;

const LDAP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Authentication {
    pub ldap_address: String,
    pub user_directory: String,
    pub rdn_identifier: String,
    pub recursive: bool,
}

// HTTP Basic Auth according to RFC 7617
#[async_trait]
impl Middleware for Authentication {
    async fn modify_request(
        &self,
        request: Request<Body>,
        _context: &Context<'_>,
    ) -> Result<Request<Body>, Response<Body>> {
        if self.user_authentication(request.headers()).await.is_some() {
            Ok(request)
        } else {
            Err(response_unauthorized())
        }
    }
}

impl Authentication {
    async fn user_authentication(&self, headers: &HeaderMap) -> Option<()> {
        let auth_data = headers.get(AUTHORIZATION)?.to_str().ok()?;
        let credentials = Credentials::from_header(auth_data.to_string()).ok()?;
        // An empty password turns an LDAP simple bind into an unauthenticated
        // bind, which most servers accept: never treat that as a login.
        if credentials.user_id.is_empty() || credentials.password.is_empty() {
            return None;
        }
        let auth_result = tokio::time::timeout(
            LDAP_TIMEOUT,
            self.check_user_credentials(&credentials.user_id, &credentials.password),
        )
        .await
        .map_err(|_| error!("LDAP authentication timed out after {:?}", LDAP_TIMEOUT))
        .ok()?
        .map_err(|e| error!("{}", e))
        .ok()?;
        if auth_result {
            Some(())
        } else {
            None
        }
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

fn response_unauthorized() -> Response<Body> {
    Response::builder()
        .header(WWW_AUTHENTICATE, "Basic realm=\"RustStrom\"")
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::empty())
        .unwrap_or_default()
}
