use crate::error_response::{bad_request, not_found};
use acme_lib::order::NewOrder;
use acme_lib::persist::FilePersist;
use acme_lib::{create_rsa_key, Certificate, Directory, DirectoryUrl, Error};
use hyper::{Body, Request, Response, StatusCode};
use std::{
  path::Path,
  sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_util::either::Either;
use tokio_util::either::Either::{Left, Right};

struct OpenChallenge {
  token: String,
  proof: String,
}

pub struct AcmeHandler {
  challenges: Arc<Mutex<Vec<OpenChallenge>>>,
}

type ChallengeSender = UnboundedSender<Either<(String, String), Result<Certificate, Error>>>;

impl AcmeHandler {
  pub fn new() -> AcmeHandler {
    AcmeHandler {
      challenges: Arc::new(Mutex::new(Vec::new())),
    }
  }

  fn add_challenge(&self, token: &str, proof: String) {
    let challenge = OpenChallenge {
      token: token.to_string(),
      proof,
    };
    let mut challenges = self.challenges.lock().unwrap();
    challenges.push(challenge);
  }

  fn get_proof_for_challenge(&self, token: &str) -> Option<String> {
    let challenges = self.challenges.lock().unwrap();
    challenges.iter().find(|c| c.token == token).map(|c| c.proof.clone())
  }

  fn remove_challenge(&self, token: &str) {
    let mut challenges = self.challenges.lock().unwrap();
    challenges
      .iter()
      .position(|c| c.token == token)
      .map(|i| challenges.remove(i));
  }

  fn start_challenge_handler(ord_new: NewOrder<FilePersist>, cs: ChallengeSender) {
    // TODO maybe add own async implementation of the acme lib so we can use an async fn instead of a thread
    fn generate_and_validate_challenge(
      mut ord_new: NewOrder<FilePersist>,
      cs: &ChallengeSender,
    ) -> Result<Certificate, Error> {
      loop {
        if let Some(ord_csr) = ord_new.confirm_validations() {
          let pkey_pri = create_rsa_key(4096);
          let ord_cert = ord_csr.finalize_pkey(pkey_pri, 5000)?;
          return ord_cert.download_and_save_cert();
        }

        let auths = ord_new.authorizations()?;
        let chall = auths[0].http_challenge();
        let token = chall.http_token().to_string();
        let proof = chall.http_proof();
        cs.send(Left((token, proof)))
          .map_err(|e| acme_lib::Error::Other(e.to_string()))?;

        chall.validate(5000)?;
        ord_new.refresh()?;
      }
    }

    std::thread::spawn(move || {
      // TODO add retry
      let result = generate_and_validate_challenge(ord_new, &cs);
      let _ = cs.send(Right(result));
    });
  }

  pub async fn initiate_challenge<P: AsRef<Path>>(
    &self,
    staging: bool,
    persist_dir: P,
    email: &str,
    primary_name: &str,
  ) -> Result<Certificate, Error> {
    std::fs::create_dir_all(&persist_dir).map_err(|e| Error::Other(e.to_string()))?;
    let persist = FilePersist::new(&persist_dir);
    let dir_url = if staging {
      DirectoryUrl::LetsEncryptStaging
    } else {
      DirectoryUrl::LetsEncrypt
    };
    let dir = Directory::from_url(persist, dir_url)?;
    let acc = dir.account(email)?;

    let existing_cert = acc.certificate(primary_name)?;
    if let Some(cert) = existing_cert {
      if cert.valid_days_left() > 0 {
        return Ok(cert);
      }
    }

    let ord_new = acc.new_order(primary_name, &[])?;
    let (cs, mut cr) = unbounded_channel();
    AcmeHandler::start_challenge_handler(ord_new, cs);

    let mut result = cr.recv().await.unwrap();
    loop {
      match result {
        Left((token, proof)) => {
          self.add_challenge(&token, proof);
          result = cr.recv().await.unwrap();
          self.remove_challenge(&token);
        }
        Right(cert) => return cert,
      }
    }
  }

  fn is_challenge(&self, request: &Request<Body>) -> bool {
    request.uri().path().starts_with("/.well-known/acme-challenge/")
  }

  pub fn respond_to_challenge(&self, request: &Request<Body>) -> Option<Response<Body>> {
    if !self.is_challenge(&request) {
      None
    } else {
      Some(request.uri().path().split('/').last().map_or_else(
        || bad_request("Unable to extract token from last path param!"),
        |token| {
          self.get_proof_for_challenge(token).map_or_else(not_found, |proof| {
            Response::builder()
              .status(StatusCode::OK)
              .body(Body::from(proof))
              .unwrap()
          })
        },
      ))
    }
  }
}