//! One connection's subscriptions and identity.
//!
//! Pure, and separate from the socket that owns it, so the rule that matters
//! most here — a private update reaches exactly the user it belongs to — is
//! decided by a function a test can call directly.

use cex_auth::Tokens;
use cex_proto::UserId;
use std::collections::BTreeSet;

use crate::route::Update;
use crate::wire::Channel;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("channel {0} requires an authenticated connection")]
    AuthRequired(String),
    #[error("invalid or expired token")]
    BadToken,
}

#[derive(Debug, Default)]
pub struct Session {
    user: Option<UserId>,
    subscriptions: BTreeSet<Channel>,
}

impl Session {
    pub fn new() -> Self {
        Session::default()
    }

    pub fn user(&self) -> Option<UserId> {
        self.user
    }

    pub fn subscriptions(&self) -> Vec<String> {
        self.subscriptions.iter().map(|c| c.to_string()).collect()
    }

    /// Attach an identity to this connection.
    ///
    /// A rejected token clears whatever identity was already attached. Leaving
    /// the previous one in place would mean a failed takeover attempt silently
    /// leaves the caller on the feed they were already on — which is fine when
    /// it is the same person retrying, and a private feed handed to the wrong
    /// person when it is not. Clearing is the only answer that is safe in both.
    pub fn authenticate(&mut self, tokens: &Tokens, token: &str) -> Result<UserId, SessionError> {
        match tokens.verify(token) {
            Ok(user) => {
                self.user = Some(user);
                Ok(user)
            }
            Err(_) => {
                self.user = None;
                Err(SessionError::BadToken)
            }
        }
    }

    pub fn subscribe(&mut self, channel: Channel) -> Result<(), SessionError> {
        if channel.is_private() && self.user.is_none() {
            return Err(SessionError::AuthRequired(channel.to_string()));
        }
        self.subscriptions.insert(channel);
        Ok(())
    }

    pub fn unsubscribe(&mut self, channel: &Channel) {
        self.subscriptions.remove(channel);
    }

    /// Whether this connection should be sent `update`.
    ///
    /// Two conditions, and both are checked every time. The audience check does
    /// not trust `subscribe` to have already refused an unauthenticated private
    /// subscription: this is the last gate before bytes reach a socket, so it
    /// re-decides rather than assuming.
    pub fn wants(&self, update: &Update) -> bool {
        if !self.subscriptions.contains(&update.channel) {
            return false;
        }
        match update.audience {
            None => true,
            Some(target) => self.user == Some(target),
        }
    }
}
