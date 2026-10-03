//! Frozen Go/Web wire contract. Go's optional zero values and base64 bytes are explicit.
pub mod core;
mod generated;
pub use generated::*;
use serde::{Deserialize, Deserializer};

pub const PTY_REPLAY_BUFFER_LIMIT: usize = 2 * 1024 * 1024;
pub const TYPE_SESSION_DISMISSED: &str = "session_dismissed";
pub const RELAY_BRANCH_PREFIX: &str = "many-ai-cli/relay/";

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
    v == &T::default()
}
fn is_false(v: &bool) -> bool {
    !v
}

mod base64_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(value))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Bytes {
            Text(String),
            Array(Vec<u8>),
        }
        match Option::<Bytes>::deserialize(deserializer)? {
            None => Ok(Vec::new()),
            Some(Bytes::Array(bytes)) => Ok(bytes),
            Some(Bytes::Text(s)) => STANDARD
                .decode(s.replace(['\r', '\n'], ""))
                .map_err(D::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn false_activity_is_explicit_but_message_flags_omit_zero() {
        let message = Message {
            r#type: "activity".into(),
            activity: Some(SessionActivity::default()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            json!({"type":"activity","token_statusbar":false,"activity":{"output_idle":false,"workflow_active":false,"awaiting_user":false,"awaiting_approval":false}})
        );
    }
    #[test]
    fn go_byte_contract_and_unknown_fields() {
        for input in [
            json!({"data":"AP8=","future_field":"ignored"}),
            json!({"data":[0,255]}),
        ] {
            let m: Message = serde_json::from_value(input).unwrap();
            assert_eq!(m.data, [0, 255]);
            assert_eq!(
                serde_json::to_value(m).unwrap(),
                json!({"type":"","token_statusbar":false,"data":"AP8="})
            );
        }
        assert!(serde_json::from_value::<Message>(json!({"data":"bad!"})).is_err());
    }
    #[test]
    fn null_scalar_and_pointer_false_follow_go() {
        let m: Message =
            serde_json::from_value(json!({"type":null,"rows":null,"binary_stale":false})).unwrap();
        assert_eq!(
            serde_json::to_value(m).unwrap(),
            json!({"type":"","token_statusbar":false,"binary_stale":false})
        );
    }
    #[test]
    fn required_slice_distinguishes_null_and_empty() {
        let a: WfPhase = serde_json::from_value(json!({"title":"t","agents":null})).unwrap();
        let b: WfPhase = serde_json::from_value(json!({"title":"t","agents":[]})).unwrap();
        assert_ne!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(b).unwrap()
        );
    }
}
