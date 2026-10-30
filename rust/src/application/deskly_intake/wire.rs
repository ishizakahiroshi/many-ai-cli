//! Closed decoders for the Deskly stage-4 PC-relay MCP results. Shapes follow
//! Deskly PR #132 `agent-development-pc-relay-model.ts` and the tool handlers'
//! own closed checks; anything else is refused instead of defaulted.
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

pub const DISCOVERY_PROFILE: &str = "pc_relay_discovery_v1";
pub const HANDOFF_PROFILE: &str = "pc_relay_handoff_v1";
pub const VALIDATION_PROFILE: &str = "pc_relay_issued_validation_v1";
/// Deskly exposes at most 16 eligible items per discovery read.
pub const MAX_DISCOVERY_ITEMS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(pub &'static str);

/// Deskly's UUID pattern: lowercase, version 1-8, RFC variant.
pub fn valid_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            14 => (b'1'..=b'8').contains(byte),
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b'),
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
}
fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub fn valid_sha1(value: &str) -> bool {
    lower_hex(value, 40)
}
pub fn valid_sha256(value: &str) -> bool {
    lower_hex(value, 64)
}
fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 2048
        && !value
            .chars()
            .any(|c| (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)))
}
/// `^#[A-Z][A-Z0-9]{0,15}-[0-9]{8}-[0-9]{3,12}$`
fn valid_number(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('#') else {
        return false;
    };
    let mut parts = rest.split('-');
    let (Some(prefix), Some(date), Some(serial), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    prefix.len() <= 16
        && prefix
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && date.len() == 8
        && date.bytes().all(|b| b.is_ascii_digit())
        && (3..=12).contains(&serial.len())
        && serial.bytes().all(|b| b.is_ascii_digit())
}
/// Deskly emits `YYYY-MM-DDTHH:MM:SS(.mmm)?Z`; returns Unix milliseconds.
pub fn parse_time_ms(value: &str) -> Option<i64> {
    let shape = matches!(value.len(), 20 | 24)
        && value.ends_with('Z')
        && value.as_bytes().get(10) == Some(&b'T');
    if !shape {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}
fn positive(value: u64) -> Result<u64, WireError> {
    if value == 0 || value > (1 << 53) {
        return Err(WireError("non-positive or unsafe integer"));
    }
    Ok(value)
}

/// Advisory only: never a claim, lease, PR binding or start permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub request_id: String,
    pub expected_revision: u64,
    pub authorization_epoch: u64,
    pub session_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    pub work: Vec<Candidate>,
    pub truncated: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDiscovery {
    profile: String,
    work: Vec<RawCandidate>,
    truncated: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidate {
    request_id: String,
    expected_revision: u64,
    generation: u64,
    authorization_epoch: u64,
    session_id: String,
}
pub fn decode_discovery(value: Value) -> Result<Discovery, WireError> {
    let raw: RawDiscovery =
        serde_json::from_value(value).map_err(|_| WireError("discovery shape"))?;
    if raw.profile != DISCOVERY_PROFILE {
        return Err(WireError("discovery profile"));
    }
    if raw.work.len() > MAX_DISCOVERY_ITEMS {
        return Err(WireError("discovery batch too large"));
    }
    let mut work = Vec::with_capacity(raw.work.len());
    for item in raw.work {
        if !valid_uuid(&item.request_id) || item.generation != 1 || !valid_text(&item.session_id) {
            return Err(WireError("discovery candidate"));
        }
        // A batch naming one request twice is not a coherent snapshot.
        if work
            .iter()
            .any(|seen: &Candidate| seen.request_id == item.request_id)
        {
            return Err(WireError("duplicate discovery candidate"));
        }
        work.push(Candidate {
            request_id: item.request_id,
            expected_revision: positive(item.expected_revision)?,
            authorization_epoch: positive(item.authorization_epoch)?,
            session_id: item.session_id,
        });
    }
    Ok(Discovery {
        work,
        truncated: raw.truncated,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Assigned,
    Readback,
    Wired,
    Sending,
    ConsumedUnknown,
    Stopped,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub request_id: String,
    pub number: String,
    pub revision: u64,
    pub generation: u64,
    pub stage: Stage,
}
impl Receipt {
    fn check(self, request_id: &str) -> Result<Self, WireError> {
        if self.request_id != request_id
            || !valid_number(&self.number)
            || self.generation != 1
            || positive(self.revision).is_err()
        {
            return Err(WireError("receipt"));
        }
        Ok(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewToken {
    pub expires_at: String,
    pub signature: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub principal_id: String,
    pub instance: String,
    pub key_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    pub request_id: String,
    pub number: String,
    pub generation: u64,
    pub worker: Identity,
    pub orchestrator: Identity,
    pub session_id: String,
    pub destination: Value,
    pub body_digest: String,
    pub wire_digest: String,
    pub authority_expires_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub receipt: Receipt,
    pub confirmation: Option<Confirmation>,
    pub token: PreviewToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPreview {
    applied: bool,
    preview: Receipt,
    #[serde(default)]
    confirmation: Option<Confirmation>,
    preview_token: PreviewToken,
}
/// `handoff` previews must carry the confirmation; `claim` previews must not.
pub fn decode_preview(
    value: Value,
    request_id: &str,
    expect_confirmation: bool,
) -> Result<Preview, WireError> {
    let raw: RawPreview = serde_json::from_value(value).map_err(|_| WireError("preview shape"))?;
    if raw.applied
        || raw.confirmation.is_some() != expect_confirmation
        || parse_time_ms(&raw.preview_token.expires_at).is_none()
        || !valid_sha256(&raw.preview_token.signature)
    {
        return Err(WireError("preview"));
    }
    let receipt = raw.preview.check(request_id)?;
    if let Some(confirmation) = &raw.confirmation
        && (confirmation.request_id != request_id
            || confirmation.number != receipt.number
            || confirmation.generation != 1
            || !valid_text(&confirmation.session_id)
            || !confirmation.destination.is_object()
            || !valid_sha256(&confirmation.body_digest)
            || !valid_sha256(&confirmation.wire_digest)
            || parse_time_ms(&confirmation.authority_expires_at).is_none())
    {
        return Err(WireError("preview confirmation"));
    }
    Ok(Preview {
        receipt,
        confirmation: raw.confirmation,
        token: raw.preview_token,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaimApplied {
    applied: bool,
    receipt: Receipt,
}
pub fn decode_claim_applied(value: Value, request_id: &str) -> Result<Receipt, WireError> {
    let raw: RawClaimApplied =
        serde_json::from_value(value).map_err(|_| WireError("claim shape"))?;
    if !raw.applied {
        return Err(WireError("claim not applied"));
    }
    raw.receipt.check(request_id)
}

/// Issued handoff with the private wire body dropped at decode time: the
/// Slack worker text is never parsed, stored or turned into a local command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedHandoff {
    pub receipt: Receipt,
    pub authorization_epoch: u64,
    pub attempt_id: String,
    pub authority_id: String,
    pub consumption_id: String,
    pub wire_id: String,
    pub wire_digest: String,
    pub session_id: String,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffApplied {
    Issued(IssuedHandoff),
    /// Authority was already consumed; never a reason to retry.
    Spent(Receipt),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHandoffApplied {
    applied: bool,
    outcome: String,
    receipt: Receipt,
    #[serde(default)]
    handoff: Option<RawHandoff>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHandoff {
    profile: String,
    request_id: String,
    number: String,
    generation: u64,
    authorization_epoch: u64,
    attempt_id: String,
    authority_id: String,
    consumption_id: String,
    wire_id: String,
    worker: Identity,
    orchestrator: Identity,
    session_id: String,
    issued_at: String,
    expires_at: String,
    wire: RawWire,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWire {
    version: u64,
    template_version: u64,
    role: String,
    transport: String,
    kind: String,
    ready_to_send: bool,
    automatic_start: bool,
    destination: Value,
    #[allow(dead_code)]
    text: IgnoredAny,
    wire_digest: String,
    marker: String,
    unfurl_links: bool,
    unfurl_media: bool,
}
pub fn decode_handoff_applied(
    value: Value,
    request_id: &str,
    attempt_id: &str,
) -> Result<HandoffApplied, WireError> {
    let raw: RawHandoffApplied =
        serde_json::from_value(value).map_err(|_| WireError("handoff shape"))?;
    if !raw.applied {
        return Err(WireError("handoff not applied"));
    }
    let receipt = raw.receipt.check(request_id)?;
    match (raw.outcome.as_str(), raw.handoff) {
        ("spent", None) => Ok(HandoffApplied::Spent(receipt)),
        ("issued", Some(h)) => {
            let w = &h.wire;
            let issued_at = parse_time_ms(&h.issued_at);
            let expires_at = parse_time_ms(&h.expires_at);
            let wire_ok = w.version == 1
                && w.template_version == 1
                && w.role == "worker"
                && w.transport == "slack_auto"
                && w.kind == "code"
                && !w.ready_to_send
                && !w.automatic_start
                && !w.unfurl_links
                && !w.unfurl_media
                && w.destination.is_object()
                && valid_sha256(&w.wire_digest)
                && valid_sha256(&w.marker);
            let ok = h.profile == HANDOFF_PROFILE
                && h.request_id == request_id
                && h.number == receipt.number
                && h.generation == 1
                && h.attempt_id == attempt_id
                && receipt.stage == Stage::ConsumedUnknown
                && [
                    &h.attempt_id,
                    &h.authority_id,
                    &h.consumption_id,
                    &h.wire_id,
                ]
                .iter()
                .all(|id| valid_uuid(id))
                && valid_text(&h.session_id)
                && [&h.worker, &h.orchestrator].iter().all(|id| {
                    valid_text(&id.principal_id)
                        && valid_text(&id.instance)
                        && valid_text(&id.key_id)
                })
                && matches!((issued_at, expires_at), (Some(a), Some(b)) if b > a)
                && wire_ok;
            if !ok {
                return Err(WireError("issued handoff"));
            }
            Ok(HandoffApplied::Issued(IssuedHandoff {
                receipt,
                authorization_epoch: positive(h.authorization_epoch)?,
                attempt_id: h.attempt_id,
                authority_id: h.authority_id,
                consumption_id: h.consumption_id,
                wire_id: h.wire_id,
                wire_digest: h.wire.wire_digest,
                session_id: h.session_id,
                expires_at_ms: expires_at.unwrap_or_default(),
            }))
        }
        _ => Err(WireError("handoff outcome")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssuedValidation {
    pub profile: String,
    pub request_id: String,
    pub revision: u64,
    pub generation: u64,
    pub authorization_epoch: u64,
    pub client_attempt_id: String,
    pub authority_id: String,
    pub consumption_id: String,
    pub wire_id: String,
    pub wire_digest: String,
    pub session_id: String,
    pub validated_at: String,
    pub valid_until: String,
}
/// Validation is data, never an execution grant: the caller still compares
/// every field to the issued handoff it holds and checks `valid_until`.
pub fn decode_validation(value: Value, issued: &IssuedHandoff) -> Result<i64, WireError> {
    let raw: IssuedValidation =
        serde_json::from_value(value).map_err(|_| WireError("validation shape"))?;
    let until = parse_time_ms(&raw.valid_until);
    let ok = raw.profile == VALIDATION_PROFILE
        && raw.request_id == issued.receipt.request_id
        && raw.generation == 1
        && raw.authorization_epoch == issued.authorization_epoch
        && raw.client_attempt_id == issued.attempt_id
        && raw.authority_id == issued.authority_id
        && raw.consumption_id == issued.consumption_id
        && raw.wire_id == issued.wire_id
        && raw.wire_digest == issued.wire_digest
        && raw.session_id == issued.session_id
        && positive(raw.revision).is_ok()
        && parse_time_ms(&raw.validated_at).is_some();
    match (ok, until) {
        (true, Some(until)) => Ok(until),
        _ => Err(WireError("issued validation")),
    }
}
