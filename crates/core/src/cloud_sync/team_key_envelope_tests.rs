use super::team_key_envelope::{
    MIN_NEW_TEAM_KEY_CHARS, TeamKeyEnvelopeError, TeamKeyKdfParams, TeamKeyScheme,
    create_team_key_envelope, detect_team_key_scheme, unlock_team_key,
};
use crate::crypto;

const PASSPHRASE: &str = "correct horse battery staple";

#[test]
fn v2_envelopes_use_random_salt_and_data_keys() {
    let params = TeamKeyKdfParams::for_tests();

    let first = create_team_key_envelope(PASSPHRASE, params).expect("first envelope");
    let second = create_team_key_envelope(PASSPHRASE, params).expect("second envelope");

    assert_ne!(first.verification, second.verification);
    assert_ne!(first.data_key, second.data_key);
    assert_eq!(
        TeamKeyScheme::EnvelopeV2,
        detect_team_key_scheme(&first.verification)
    );
}

#[test]
fn v2_envelope_unlocks_only_with_the_correct_passphrase() {
    let created = create_team_key_envelope(PASSPHRASE, TeamKeyKdfParams::for_tests())
        .expect("create envelope");

    let unlocked = unlock_team_key(&created.verification, PASSPHRASE).expect("unlock envelope");
    let wrong = unlock_team_key(&created.verification, "wrong team passphrase");

    assert_eq!(TeamKeyScheme::EnvelopeV2, unlocked.scheme);
    assert_eq!(created.data_key, unlocked.data_key);
    assert_eq!(Err(TeamKeyEnvelopeError::InvalidKeyOrEnvelope), wrong);
}

#[test]
fn v2_envelope_rejects_tampering() {
    let created = create_team_key_envelope(PASSPHRASE, TeamKeyKdfParams::for_tests())
        .expect("create envelope");
    let mut tampered = created.verification;
    let replacement = if tampered.ends_with('A') { 'B' } else { 'A' };
    tampered.pop();
    tampered.push(replacement);

    assert_eq!(
        Err(TeamKeyEnvelopeError::InvalidKeyOrEnvelope),
        unlock_team_key(&tampered, PASSPHRASE)
    );
}

#[test]
fn legacy_verification_remains_readable() {
    let verification = crypto::generate_key_verification("legacy-key");

    let unlocked = unlock_team_key(&verification, "legacy-key").expect("unlock legacy key");

    assert_eq!(TeamKeyScheme::Legacy, detect_team_key_scheme(&verification));
    assert_eq!(TeamKeyScheme::Legacy, unlocked.scheme);
    assert_eq!("legacy-key", unlocked.data_key);
}

#[test]
fn new_team_passphrases_require_twelve_characters() {
    let error = create_team_key_envelope("short", TeamKeyKdfParams::for_tests())
        .expect_err("short passphrase must fail");

    assert_eq!(
        TeamKeyEnvelopeError::KeyTooShort {
            minimum: MIN_NEW_TEAM_KEY_CHARS,
        },
        error
    );
}
