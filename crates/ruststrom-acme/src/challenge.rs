use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
};

/// URL path prefix under which the CA fetches HTTP-01 challenge responses.
pub const CHALLENGE_PATH_PREFIX: &str = "/.well-known/acme-challenge/";

/// Pending HTTP-01 challenge responses, keyed by token.
///
/// The [`AcmeManager`](crate::AcmeManager) inserts a token while an order is being validated
/// and removes it afterwards. The HTTP listener (port 80) answers
/// `GET /.well-known/acme-challenge/<token>` with [`ChallengeStore::get`] as a `text/plain`
/// body, or 404 when it returns `None`.
///
/// Cloning is cheap (one `Arc`); all clones share the same entries. Safe to use from any thread.
#[derive(Clone, Default, Debug)]
pub struct ChallengeStore {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl ChallengeStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the key authorization to serve for `token`, if a challenge is pending.
    pub fn get(&self, token: &str) -> Option<String> {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(token)
            .cloned()
    }

    /// Convenience for request handlers: returns the response body for a request path such as
    /// `/.well-known/acme-challenge/<token>`, or `None` if the path is not a challenge path or
    /// the token is unknown.
    pub fn response_for_path(&self, path: &str) -> Option<String> {
        let token = path.strip_prefix(CHALLENGE_PATH_PREFIX)?;
        if token.is_empty() || token.contains('/') {
            return None;
        }
        self.get(token)
    }

    /// Number of pending challenges.
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// True when no challenge is pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn insert(&self, token: String, key_authorization: String) {
        self.inner
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(token, key_authorization);
    }

    pub(crate) fn remove(&self, token: &str) {
        self.inner
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(token);
    }

    /// Inserts a challenge that is removed again when the returned guard is dropped, so tokens
    /// do not leak when an order fails, times out or its future is cancelled.
    pub(crate) fn publish(&self, token: String, key_authorization: String) -> ChallengeGuard {
        self.insert(token.clone(), key_authorization);
        ChallengeGuard {
            store: self.clone(),
            token,
        }
    }
}

/// Removes its token from the [`ChallengeStore`] on drop.
pub(crate) struct ChallengeGuard {
    store: ChallengeStore,
    token: String,
}

impl Drop for ChallengeGuard {
    fn drop(&mut self) {
        self.store.remove(&self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_removes_token() {
        let store = ChallengeStore::new();
        let clone = store.clone();
        {
            let _guard = store.publish("tok".into(), "tok.thumb".into());
            assert_eq!(clone.get("tok").as_deref(), Some("tok.thumb"));
            assert_eq!(
                clone
                    .response_for_path("/.well-known/acme-challenge/tok")
                    .as_deref(),
                Some("tok.thumb")
            );
        }
        assert!(clone.is_empty());
        assert_eq!(clone.get("tok"), None);
    }

    #[test]
    fn response_for_path_rejects_other_paths() {
        let store = ChallengeStore::new();
        store.insert("tok".into(), "v".into());
        assert_eq!(store.response_for_path("/tok"), None);
        assert_eq!(
            store.response_for_path("/.well-known/acme-challenge/"),
            None
        );
        assert_eq!(
            store.response_for_path("/.well-known/acme-challenge/a/tok"),
            None
        );
    }
}
