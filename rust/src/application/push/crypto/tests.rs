use super::*;
fn fixture_store() -> Store {
    let mut private = [0; 32];
    private[31] = 3;
    let key = SecretKey::from_slice(&private).unwrap();
    Store {
        vapid_private_key: URL_SAFE_NO_PAD.encode(private),
        vapid_public_key: URL_SAFE_NO_PAD
            .encode(key.public_key().to_encoded_point(false).as_bytes()),
        ..Default::default()
    }
}
#[test]
fn source_go_ecdh_hkdf_aes128gcm_ciphertext_matches_independent_vectors() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/web-push/go-encryption-vectors.json"
    ))
    .unwrap();
    let mut private = [0; 32];
    private[31] = 2;
    let key = SecretKey::from_slice(&private).unwrap();
    for row in cases.as_array().unwrap() {
        let encoded = encode_with(
            &fixture_store(),
            &Keys {
                auth: row["auth"].as_str().unwrap().into(),
                p256dh: row["p256dh"].as_str().unwrap().into(),
            },
            "https://example.invalid/push",
            row["payload"].as_str().unwrap().as_bytes(),
            "synthetic".into(),
            1791158400,
            (&key, [0; 16]),
        )
        .unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(encoded.body),
            row["body"].as_str().unwrap()
        );
    }
}
#[test]
fn vapid_jwt_signature_and_source_double_mailto_claim_are_real() {
    use p256::ecdsa::{VerifyingKey, signature::Verifier};
    let store = fixture_store();
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/web-push/go-encryption-vectors.json"
    ))
    .unwrap();
    let row = &cases[0];
    let encoded = encode(
        &store,
        &Keys {
            auth: row["auth"].as_str().unwrap().into(),
            p256dh: row["p256dh"].as_str().unwrap().into(),
        },
        "https://example.invalid:443/push",
        b"synthetic",
        "synthetic".into(),
        1791158400,
    )
    .unwrap();
    let jwt = encoded
        .authorization
        .strip_prefix("vapid t=")
        .unwrap()
        .split_once(", k=")
        .unwrap()
        .0;
    let parts = jwt.split('.').collect::<Vec<_>>();
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(claims["aud"], "https://example.invalid:443");
    assert_eq!(claims["exp"], 1791201600i64);
    assert_eq!(
        claims["sub"],
        format!("mailto:mailto:{}@{}", "many-ai-cli", "localhost.invalid")
    );
    let verifier =
        VerifyingKey::from_sec1_bytes(&decode_vapid(&store.vapid_public_key).unwrap()).unwrap();
    let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
    assert!(
        verifier
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
            .is_ok()
    );
    assert!(verifier.verify(b"tampered message", &sig).is_err());
}
#[test]
fn base64_matches_go_noncanonical_trailing_bits_and_embedded_newlines() {
    assert_eq!(decode_subscription("AB==").unwrap(), vec![0]);
    assert_eq!(decode_subscription("A\r\nA==").unwrap(), vec![0]);
    assert!(decode_subscription("!").is_err());
}
