use super::{Context, Middleware};
use async_trait::async_trait;
use http_auth_basic::Credentials;
use hyper::{
  header::{AUTHORIZATION, WWW_AUTHENTICATE},
  Body, HeaderMap, Request, Response, StatusCode,
};
use ldap3::{LdapConnAsync, LdapError, Scope, SearchEntry};
use log::error;

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
    let auth_result = self
      .check_user_credentials(&credentials.user_id, &credentials.password)
      .await
      .map_err(|e| error!("{}", e))
      .ok()?;
    if auth_result {
      Some(())
    } else {
      None
    }
  }

  async fn check_user_credentials(&self, user: &str, password: &str) -> Result<bool, LdapError> {
    let (conn, mut ldap) = LdapConnAsync::new(&self.ldap_address).await?;
    ldap3::drive!(conn);
    let scope = if self.recursive {
      Scope::Subtree
    } else {
      Scope::OneLevel
    };
    let filter = format!("({}={})", self.rdn_identifier, user);
    let (result_entry, _) = ldap
      .search(&self.user_directory, scope, &filter, vec!["1.1"])
      .await?
      .success()?;

    for entry in result_entry {
      let sn = SearchEntry::construct(entry);
      let bind_user = ldap.simple_bind(&sn.dn, password).await?;
      if bind_user.success().is_ok() {
        return Ok(true);
      }
    }
    Ok(false)
  }
}

fn response_unauthorized() -> Response<Body> {
  Response::builder()
    .header(
      WWW_AUTHENTICATE,
      "Basic realm=\"Another Rust Load Balancer requires authentication\"",
    )
    .status(StatusCode::UNAUTHORIZED)
    .body(Body::empty())
    .unwrap()
}