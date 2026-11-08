//! Time-based one-time passwords, RFC 6238.
//!
//! RustCrypto HMAC-SHA1 with the published SHA1 test vectors below.
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Seconds per code. The interval every authenticator app assumes.
pub const STEP: u64 = 30;
/// Digits in a code.
pub const DIGITS: u32 = 6;
/// How many steps either side of now are accepted.
///
/// One step back and forward, so a code entered as it rolls over still works
/// and a clock a few seconds out does not lock an owner out of their own
/// console. Wider than this starts to matter: each extra step is another code
/// an attacker may guess.
pub const SKEW_STEPS: u64 = 1;

/// A new random secret, base32 as authenticator apps expect.
///
/// The caller supplies 20 random bytes (the RFC 4226 recommended key size).
/// Keeping randomness at the call site also permits deterministic tests.
pub fn encode_secret(bytes: &[u8]) -> String {
    BASE32_NOPAD.encode(bytes)
}

/// Decode a base32 secret, accepting the lowercase and padded forms people
/// paste. `None` when it is not base32 at all.
pub fn decode_secret(secret: &str) -> Option<Vec<u8>> {
    let cleaned: String = secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .flat_map(char::to_uppercase)
        .collect();
    BASE32_NOPAD.decode(cleaned.as_bytes()).ok()
}

/// The code for one counter value.
fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // Dynamic truncation, RFC 4226 §5.3: the low nibble of the last byte
    // picks where in the digest to read the code from.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    binary % 10u32.pow(DIGITS)
}

/// The code for a Unix time, zero-padded to [`DIGITS`].
pub fn code_at(secret: &[u8], unix_seconds: u64) -> String {
    format!(
        "{:0width$}",
        hotp(secret, unix_seconds / STEP),
        width = DIGITS as usize
    )
}

/// Which time step a code matches, if any.
///
/// The step is returned so a caller can refuse a code it has already
/// accepted: within one 30-second window the same six digits stay valid, so
/// someone who watched them being typed could otherwise reuse them. Storing
/// the last accepted step makes each code single-use.
pub fn matching_step(secret: &[u8], candidate: &str, unix_seconds: u64) -> Option<u64> {
    let candidate: String = candidate.chars().filter(|c| !c.is_whitespace()).collect();
    if candidate.len() != DIGITS as usize || !candidate.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = unix_seconds / STEP;
    let mut matched = None;
    for offset in 0..=(SKEW_STEPS * 2) {
        let step = now.saturating_sub(SKEW_STEPS).saturating_add(offset);
        let expected = format!("{:0width$}", hotp(secret, step), width = DIGITS as usize);
        // Every step is checked even once one matches, so the time taken does
        // not reveal which window a code came from.
        if crate::constant_time_eq(&expected, &candidate) {
            matched = Some(step);
        }
    }
    matched
}

/// Does `candidate` match the secret at this time, within the accepted skew?
///
/// Compared in constant time with respect to the digits, so a wrong code
/// gives nothing away by how long it took to reject.
pub fn verify(secret: &[u8], candidate: &str, unix_seconds: u64) -> bool {
    matching_step(secret, candidate, unix_seconds).is_some()
}

/// The `otpauth://` URI an authenticator app reads from a QR code.
///
/// Label and issuer are percent-encoded, because an operator's email
/// contains characters that would otherwise end the path early.
pub fn provisioning_uri(issuer: &str, account: &str, secret: &str) -> String {
    let encode = |value: &str| {
        value
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                other => format!("%{other:02X}"),
            })
            .collect::<String>()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        encode(issuer),
        encode(account),
        encode(secret),
        encode(issuer),
        DIGITS,
        STEP
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238's published vectors for HMAC-SHA1, truncated to six digits.
    /// The whole implementation stands or falls on these.
    #[test]
    fn matches_the_rfc_6238_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59u64, "287082"),
            (1111111109, "081804"),
            (1111111111, "050471"),
            (1234567890, "005924"),
            (2000000000, "279037"),
            (20000000000, "353130"),
        ] {
            assert_eq!(code_at(secret, time), expected, "at {time}");
        }
    }

    #[test]
    fn accepts_the_neighbouring_step_and_rejects_further() {
        let secret = b"12345678901234567890";
        let now = 1111111111;
        assert!(verify(secret, &code_at(secret, now), now));
        assert!(verify(secret, &code_at(secret, now - STEP), now));
        assert!(verify(secret, &code_at(secret, now + STEP), now));
        assert!(!verify(secret, &code_at(secret, now - STEP * 3), now));
        assert!(!verify(secret, &code_at(secret, now + STEP * 3), now));
    }

    /// The step a code belongs to is what makes it single-use.
    #[test]
    fn reports_which_step_matched() {
        let secret = b"12345678901234567890";
        let now = 1111111111;
        assert_eq!(
            matching_step(secret, &code_at(secret, now), now),
            Some(now / STEP)
        );
        assert_eq!(
            matching_step(secret, &code_at(secret, now - STEP), now),
            Some(now / STEP - 1)
        );
        // A code from outside the window belongs to no accepted step.
        assert_eq!(
            matching_step(secret, &code_at(secret, now + STEP * 5), now),
            None
        );
    }

    #[test]
    fn rejects_malformed_codes() {
        let secret = b"12345678901234567890";
        for bad in ["", "12345", "1234567", "abcdef", "12 34 5", "-00001"] {
            assert!(!verify(secret, bad, 1111111111), "accepted {bad:?}");
        }
        // Spaces inside a six-digit code are stripped, as people paste them.
        let now = 1111111111;
        let spaced = format!(
            "{} {}",
            &code_at(secret, now)[..3],
            &code_at(secret, now)[3..]
        );
        assert!(verify(secret, &spaced, now));
    }

    #[test]
    fn secrets_round_trip_through_base32() {
        let bytes: Vec<u8> = (0u8..20).collect();
        let encoded = encode_secret(&bytes);
        assert_eq!(decode_secret(&encoded).as_deref(), Some(bytes.as_slice()));
        // The forms people paste: lowercase, padded, spaced.
        assert_eq!(
            decode_secret(&encoded.to_lowercase()).as_deref(),
            Some(bytes.as_slice())
        );
        assert_eq!(
            decode_secret(&format!("{encoded}===")).as_deref(),
            Some(bytes.as_slice())
        );
        assert!(decode_secret("not base32 ∑").is_none());
    }

    #[test]
    fn the_provisioning_uri_escapes_the_account() {
        let uri = provisioning_uri("Vuuut Click", "owner+test@example.com", "ABCDEF");
        assert!(
            uri.contains("Vuuut%20Click:owner%2Btest%40example.com"),
            "{uri}"
        );
        assert!(uri.contains("secret=ABCDEF"));
        assert!(uri.contains("digits=6") && uri.contains("period=30"));
    }
}
