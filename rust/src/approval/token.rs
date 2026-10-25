//! Purpose-limited one-tap HMAC tokens. Verification is read-only; nonce
//! consumption belongs after successful bound input, never GET or failed send.
use crate::proto::time::Timestamp;
use crate::proto::{
    self,
    core::{ApprovalSourceEpoch, LiveSessionId, SessionBinding, SessionIncarnation},
    wire::{Field, GoWire, Schema},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
pub const ONE_TAP_TTL: Duration = Duration::from_secs(120);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    Invalid,
    Expired,
    Consumed,
    RandomUnavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneTapAction {
    Approve,
    Reject,
}
impl OneTapAction {
    fn wire(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Claim {
    v: i64,
    sid: i64,
    inc: u64,
    aid: String,
    sig: String,
    ep: u64,
    act: String,
    exp: i64,
    n: String,
}
impl GoWire for Claim {
    const GO_TYPE: &'static str = "OneTapApprovalClaim";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "OneTapApprovalClaim",
        fields: &[
            Field {
                name: "v",
                kind: "int",
            },
            Field {
                name: "sid",
                kind: "int",
            },
            Field {
                name: "inc",
                kind: "uint64",
            },
            Field {
                name: "aid",
                kind: "string",
            },
            Field {
                name: "sig",
                kind: "string",
            },
            Field {
                name: "ep",
                kind: "uint64",
            },
            Field {
                name: "act",
                kind: "string",
            },
            Field {
                name: "exp",
                kind: "int64",
            },
            Field {
                name: "n",
                kind: "string",
            },
        ],
    }];
}
#[derive(Clone)]
pub struct VerifiedClaim(Claim);
impl VerifiedClaim {
    pub fn session(&self) -> LiveSessionId {
        LiveSessionId(self.0.sid)
    }
    pub fn incarnation(&self) -> SessionIncarnation {
        SessionIncarnation(self.0.inc)
    }
    pub fn approval_id(&self) -> &str {
        &self.0.aid
    }
    pub fn signature(&self) -> &str {
        &self.0.sig
    }
    pub fn epoch(&self) -> ApprovalSourceEpoch {
        ApprovalSourceEpoch(self.0.ep)
    }
    pub fn action(&self) -> OneTapAction {
        if self.0.act == "approve" {
            OneTapAction::Approve
        } else {
            OneTapAction::Reject
        }
    }
    pub fn nonce(&self) -> &str {
        &self.0.n
    }
}
pub struct OneTapManager {
    secret: [u8; 32],
    used: Mutex<BTreeMap<String, i64>>,
}
impl OneTapManager {
    pub fn new() -> Result<Self, TokenError> {
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).map_err(|_| TokenError::RandomUnavailable)?;
        Ok(Self {
            secret,
            used: Mutex::new(BTreeMap::new()),
        })
    }
    fn mac(&self, bytes: &[u8]) -> Hmac<Sha256> {
        let mut h = Hmac::<Sha256>::new_from_slice(&self.secret).expect("fixed HMAC key");
        h.update(bytes);
        h
    }
    pub fn issue(
        &self,
        binding: SessionBinding,
        approval_id: &str,
        signature: &str,
        epoch: ApprovalSourceEpoch,
        action: OneTapAction,
        now: Timestamp,
    ) -> Result<String, TokenError> {
        if binding.session.0 <= 0
            || binding.incarnation.0 == 0
            || approval_id.trim().is_empty()
            || signature.trim().is_empty()
            || epoch.0 == 0
        {
            return Err(TokenError::Invalid);
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|_| TokenError::RandomUnavailable)?;
        let claim = Claim {
            v: 2,
            sid: binding.session.0,
            inc: binding.incarnation.0,
            aid: approval_id.into(),
            sig: signature.into(),
            ep: epoch.0,
            act: action.wire().into(),
            exp: unix(now.checked_add(ONE_TAP_TTL).ok_or(TokenError::Invalid)?)?,
            n: nonce.iter().map(|b| format!("{b:02x}")).collect(),
        };
        let encoded = URL_SAFE_NO_PAD
            .encode(proto::provider::to_go_json(&claim).map_err(|_| TokenError::Invalid)?);
        let mac = self.mac(encoded.as_bytes()).finalize().into_bytes();
        Ok(format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(mac)))
    }
    pub fn verify(&self, token: &str, now: Timestamp) -> Result<VerifiedClaim, TokenError> {
        let parts: Vec<_> = token.split('.').collect();
        if parts.len() != 2 || parts.iter().any(|s| s.is_empty()) {
            return Err(TokenError::Invalid);
        }
        let provided = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| TokenError::Invalid)?;
        self.mac(parts[0].as_bytes())
            .verify_slice(&provided)
            .map_err(|_| TokenError::Invalid)?;
        let raw = URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| TokenError::Invalid)?;
        let claim: Claim = proto::decode_wire(&raw).map_err(|_| TokenError::Invalid)?;
        if claim.v != 2
            || claim.sid <= 0
            || claim.inc == 0
            || claim.aid.trim().is_empty()
            || claim.sig.trim().is_empty()
            || claim.ep == 0
            || claim.n.trim().is_empty()
            || !matches!(claim.act.as_str(), "approve" | "reject")
        {
            return Err(TokenError::Invalid);
        }
        if claim.exp <= unix(now)? {
            return Err(TokenError::Expired);
        }
        Ok(VerifiedClaim(claim))
    }
    /// Invoke under the live session/action transaction after transmission and
    /// binding validation. The caller must never clear a replacement candidate.
    pub fn consume(&self, claim: &VerifiedClaim, now: Timestamp) -> Result<(), TokenError> {
        let now = unix(now)?;
        let mut used = self.used.lock().unwrap_or_else(|p| p.into_inner());
        used.retain(|_, expiry| *expiry > now);
        if claim.0.exp <= now {
            return Err(TokenError::Expired);
        }
        if used.contains_key(&claim.0.n) {
            return Err(TokenError::Consumed);
        }
        used.insert(claim.0.n.clone(), claim.0.exp);
        Ok(())
    }
}
fn unix(time: Timestamp) -> Result<i64, TokenError> {
    match time.duration_since(Timestamp::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).map_err(|_| TokenError::Invalid),
        Err(e) => {
            let d = e.duration();
            let seconds = i64::try_from(d.as_secs()).map_err(|_| TokenError::Invalid)?;
            Ok(-seconds - i64::from(d.subsec_nanos() > 0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::core::WrapperConnectionId;

    #[test]
    fn correctly_signed_legacy_or_missing_incarnation_claims_are_rejected() {
        let manager = OneTapManager::new().unwrap();
        let now = Timestamp::UNIX_EPOCH + Duration::from_secs(1_000);
        let binding = SessionBinding {
            session: LiveSessionId(3),
            incarnation: SessionIncarnation(7),
            wrapper: WrapperConnectionId(1),
        };
        let current = manager
            .issue(
                binding,
                "approval",
                "sig",
                ApprovalSourceEpoch(8),
                OneTapAction::Approve,
                now,
            )
            .unwrap();
        let encoded = current.split_once('.').unwrap().0;
        let current_claim: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
        let valid = manager.verify(&current, now).unwrap();
        assert_eq!(valid.incarnation(), binding.incarnation);
        for (version, incarnation) in [(1, Some(7)), (1, None), (2, None), (2, Some(0))] {
            let mut claim = current_claim.clone();
            claim["v"] = serde_json::json!(version);
            if let Some(incarnation) = incarnation {
                claim["inc"] = serde_json::json!(incarnation);
            } else {
                claim.as_object_mut().unwrap().remove("inc");
            }
            // Sign the altered claim with the actual manager, so rejection tests
            // required semantic fields rather than merely a broken signature.
            let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claim).unwrap());
            let mac = manager.mac(encoded.as_bytes()).finalize().into_bytes();
            let token = format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(mac));
            assert!(matches!(
                manager.verify(&token, now),
                Err(TokenError::Invalid)
            ));
        }
        assert!(manager.verify(&current, now).is_ok());
        manager.consume(&valid, now).unwrap();
    }

    #[test]
    fn issuance_rejects_zero_incarnation_but_does_not_bind_wrapper_connection() {
        let manager = OneTapManager::new().unwrap();
        let now = Timestamp::UNIX_EPOCH + Duration::from_secs(1_000);
        for (incarnation, valid) in [(0, false), (9, true)] {
            let result = manager.issue(
                SessionBinding {
                    session: LiveSessionId(3),
                    incarnation: SessionIncarnation(incarnation),
                    wrapper: WrapperConnectionId(0),
                },
                "approval",
                "sig",
                ApprovalSourceEpoch(8),
                OneTapAction::Approve,
                now,
            );
            assert_eq!(result.is_ok(), valid);
            if !valid {
                assert_eq!(result, Err(TokenError::Invalid));
            }
        }
    }
}
