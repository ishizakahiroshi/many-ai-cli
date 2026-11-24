//! A provider credential is usable only while its authenticated wrapper owns
//! the live session. The signature also lets that wrapper restore the same
//! credential after a Hub restart, without recording it on disk.
use super::auth::valid_token;
use crate::proto::core::{LiveSessionId, SessionBinding};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::{collections::HashMap, io, sync::Mutex};

const PREFIX: &str = "child-v1.";
pub(crate) const REDACTION_PATTERN: &str = r"child-v1\.[0-9]{1,19}\.[0-9a-f]{64}\.[0-9a-f]{64}";

pub(crate) fn is_child(token: &str) -> bool {
    token.starts_with(PREFIX)
}

fn parts(token: &str) -> Option<(LiveSessionId, &str, &str)> {
    if token.len() > PREFIX.len() + 19 + 2 + 128 {
        return None;
    }
    let mut fields = token.strip_prefix(PREFIX)?.split('.');
    let id = fields.next()?;
    if !id.bytes().all(|b| b.is_ascii_digit()) || id.starts_with('0') {
        return None;
    }
    let session = LiveSessionId(id.parse().ok()?);
    let nonce = fields.next()?;
    let tag = fields.next()?;
    let hex = |s: &str| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    };
    (hex(nonce) && hex(tag) && fields.next().is_none()).then_some((session, nonce, tag))
}

pub(crate) fn session(token: &str) -> Option<LiveSessionId> {
    parts(token).map(|(session, _, _)| session)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn signature(master: &str, session: LiveSessionId, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(master.as_bytes())
        .expect("HMAC accepts arbitrary key lengths");
    mac.update(b"many-ai-cli/child-token/v1\0");
    mac.update(&session.0.to_be_bytes());
    mac.update(nonce.as_bytes());
    hex(&mac.finalize().into_bytes())
}

#[derive(Default)]
pub(crate) struct ChildTokens {
    // Keep only the public nonce, not the bearer credential.
    live: Mutex<HashMap<LiveSessionId, (SessionBinding, String)>>,
}

impl ChildTokens {
    fn bind(&self, binding: SessionBinding, nonce: &str) -> io::Result<()> {
        let mut live = self
            .live
            .lock()
            .map_err(|_| io::Error::other("child credential lock failed"))?;
        if live
            .get(&binding.session)
            .is_some_and(|(current, _)| current.wrapper > binding.wrapper)
        {
            return Err(io::Error::other("wrapper has been replaced"));
        }
        live.insert(binding.session, (binding, nonce.into()));
        Ok(())
    }

    pub(crate) fn issue(&self, master: &str, binding: SessionBinding) -> io::Result<String> {
        if master.is_empty() || binding.session.0 <= 0 {
            return Err(io::Error::other("missing child credential identity"));
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)
            .map_err(|_| io::Error::other("child credential randomness unavailable"))?;
        let nonce = hex(&bytes);
        let tag = signature(master, binding.session, &nonce);
        self.bind(binding, &nonce)?;
        Ok(format!("{PREFIX}{}.{nonce}.{tag}", binding.session.0))
    }

    // Called only for a wrapper which has authenticated with the Hub master.
    // A rotated master or a session renumbered during reattach cannot revive
    // an old credential held in an already running provider's environment.
    pub(crate) fn restore(
        &self,
        token: &str,
        master: &str,
        binding: SessionBinding,
    ) -> io::Result<bool> {
        let Some((id, nonce, tag)) = parts(token) else {
            return Ok(false);
        };
        if master.is_empty()
            || id != binding.session
            || !valid_token(tag, &signature(master, id, nonce))
        {
            return Ok(false);
        }
        self.bind(binding, nonce)?;
        Ok(true)
    }

    pub(crate) fn valid(&self, token: &str, master: &str, binding: SessionBinding) -> bool {
        let Some((id, nonce, tag)) = parts(token) else {
            return false;
        };
        !master.is_empty()
            && id == binding.session
            && self.live.lock().is_ok_and(|live| {
                live.get(&id)
                    .is_some_and(|(current, expected)| *current == binding && expected == nonce)
            })
            && valid_token(tag, &signature(master, id, nonce))
    }

    pub(crate) fn disconnect(&self, binding: SessionBinding) {
        if let Ok(mut live) = self.live.lock()
            && live
                .get(&binding.session)
                .is_some_and(|(current, _)| *current == binding)
        {
            live.remove(&binding.session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::core::{SessionIncarnation, WrapperConnectionId};

    fn binding(id: i64, incarnation: u64, wrapper: u64) -> SessionBinding {
        SessionBinding {
            session: LiveSessionId(id),
            incarnation: SessionIncarnation(incarnation),
            wrapper: WrapperConnectionId(wrapper),
        }
    }

    #[test]
    fn child_credential_follows_only_its_authenticated_wrapper() {
        let tokens = ChildTokens::default();
        let first = binding(1, 1, 1);
        let second = binding(2, 2, 2);
        let token = tokens.issue("first master", first).unwrap();
        assert!(
            !crate::storage::mask_secret_bytes(token.as_bytes()).starts_with(PREFIX.as_bytes())
        );
        assert!(!crate::approval::summary::mask_secrets(&token).contains(PREFIX));
        assert!(!crate::application::diagnostics::report::redact(&token).contains(PREFIX));
        assert!(tokens.valid(&token, "first master", first));
        assert!(!tokens.valid(&token, "first master", second));
        assert!(!tokens.valid(&token, "rotated master", first));
        assert!(!tokens.valid("first master", "first master", first));

        let restarted = ChildTokens::default();
        let replacement = binding(1, 3, 3);
        assert!(!restarted.valid(&token, "first master", replacement));
        assert!(
            !restarted
                .restore(&token, "rotated master", replacement)
                .unwrap()
        );
        assert!(!restarted.restore(&token, "first master", second).unwrap());
        assert!(
            restarted
                .restore(&token, "first master", replacement)
                .unwrap()
        );
        restarted.disconnect(first);
        assert!(restarted.valid(&token, "first master", replacement));
        restarted.disconnect(replacement);
        assert!(!restarted.valid(&token, "first master", replacement));

        let fresh = tokens.issue("first master", replacement).unwrap();
        assert_ne!(fresh, token);
        assert!(!tokens.valid(&token, "first master", replacement));
        assert!(tokens.valid(&fresh, "first master", replacement));
    }
}
