//! Checks offline that the release workflow of Priory's own repository built a file: GitHub's
//! attestation of it, a Sigstore bundle, against two pinned Sigstore keys.

use rustls::crypto::ring::default_provider;
use rustls::pki_types::alg_id::{ECDSA_P256, ECDSA_P384, ECDSA_SHA256, ECDSA_SHA384};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// The P-384 key of Fulcio's intermediate, which signs each workflow's certificate. `gh attestation
/// trusted-root` prints it. A rotated key refuses every release until a release pins the new one.
const FULCIO: &str = "04f11552ff2b07f8d3afb836723c866d8a581417d3656ab62901df473f5bc1047d54e4257becb492eecd19887e2713b1efee9b52e8bbef47f49393bf7c2d580cccb949e077887c5ded1d269ec4b718a52012af5912d0dfd1801273ffd8d60a25e7";
/// The P-256 key of the Rekor v1 log, which signs the time of each entry.
const REKOR: &str = "04d86d98fb6b5a6dd4d5e41706881231d1af5f005c2b9016e62d21ad92ce0bdea5fac98634cee7c19e10bc52bfe2cb9e468563fff40fdb6362e10b7d0cf7e458b7";

const ISSUER: &[u8] = b"https://token.actions.githubusercontent.com";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

const KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];
/// 1.3.6.1.4.1.57264.1.8, Fulcio's OIDC issuer.
const OIDC_ISSUER: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x08];
/// The sequence that holds code signing (1.3.6.1.5.5.7.3.3) alone.
const CODE_SIGNING: &[u8] = &[0x30, 0x0a, 0x06, 0x08, 0x2b, 6, 1, 5, 5, 7, 3, 3];

type Checked<T> = Result<T, String>;

#[derive(Deserialize)]
struct Attestations {
    attestations: Vec<Attestation>,
}

#[derive(Deserialize)]
struct Attestation {
    bundle: Bundle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    verification_material: Material,
    dsse_envelope: Envelope,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Material {
    certificate: Raw,
    tlog_entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    raw_bytes: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    log_index: String,
    log_id: KeyId,
    integrated_time: String,
    inclusion_promise: Promise,
    canonicalized_body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyId {
    key_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Promise {
    signed_entry_timestamp: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    payload: String,
    payload_type: String,
    signatures: Vec<Signature>,
}

#[derive(Deserialize)]
struct Signature {
    sig: String,
}

/// Whether GitHub's answer `attestations` proves that the release workflow of `repository`, run for
/// the tag `v<version>`, built the file `name` with the lower-case hex `sha256`.
pub(crate) fn verify(
    attestations: &[u8],
    repository: &str,
    version: &str,
    name: &str,
    sha256: &str,
) -> Checked<()> {
    let answer: Attestations = serde_json::from_slice(attestations)
        .map_err(|error| format!("the attestations do not parse: {error}"))?;
    let identity = format!(
        "https://github.com/{repository}/.github/workflows/release.yml@refs/tags/v{version}"
    );
    let mut refused = "GitHub holds no attestation of it".to_owned();
    for attestation in &answer.attestations {
        match bundle(&attestation.bundle, &identity, name, sha256) {
            Ok(()) => return Ok(()),
            Err(error) => refused = error,
        }
    }
    Err(refused)
}

fn bundle(bundle: &Bundle, identity: &str, name: &str, sha256: &str) -> Checked<()> {
    let der = base64(&bundle.verification_material.certificate.raw_bytes)?;
    let leaf = Leaf::parse(&der).ok_or("its certificate has a shape Priory does not accept")?;
    signed(true, &hex(FULCIO), leaf.signed, leaf.signature)
        .map_err(|()| "Fulcio did not sign its certificate")?;
    if leaf.issuer != ISSUER {
        return Err("its certificate is not from GitHub Actions".into());
    }
    if leaf.identity != identity.as_bytes() {
        return Err(format!(
            "another workflow or tag built it: {}",
            String::from_utf8_lossy(leaf.identity)
        ));
    }
    let envelope = &bundle.dsse_envelope;
    let [signature] = &envelope.signatures[..] else {
        return Err("its envelope does not hold one signature".into());
    };
    if envelope.payload_type != PAYLOAD_TYPE {
        return Err(format!("its payload is {}", envelope.payload_type));
    }
    let payload = base64(&envelope.payload)?;
    let mut message = format!(
        "DSSEv1 {} {PAYLOAD_TYPE} {} ",
        PAYLOAD_TYPE.len(),
        payload.len()
    )
    .into_bytes();
    message.extend_from_slice(&payload);
    signed(false, leaf.key, &message, &base64(&signature.sig)?)
        .map_err(|()| "its payload's signature is wrong")?;
    let entries = &bundle.verification_material.tlog_entries;
    let logged = entries
        .iter()
        .any(|entry| logged(entry, &leaf, &der, &payload, &signature.sig).unwrap_or(false));
    if !logged {
        return Err("no Rekor entry proves it was signed while its certificate was valid".into());
    }
    states(&payload, name, sha256)
}

/// Whether the in-toto statement `payload` is a build provenance of the file.
fn states(payload: &[u8], name: &str, sha256: &str) -> Checked<()> {
    let statement: Value = serde_json::from_slice(payload)
        .map_err(|error| format!("its statement does not parse: {error}"))?;
    if statement["_type"] != "https://in-toto.io/Statement/v1"
        || statement["predicateType"] != "https://slsa.dev/provenance/v1"
    {
        return Err("its statement is not a build provenance".into());
    }
    let named = statement["subject"].as_array().is_some_and(|subjects| {
        subjects
            .iter()
            .any(|subject| subject["name"] == name && subject["digest"]["sha256"] == sha256)
    });
    named
        .then_some(())
        .ok_or_else(|| format!("its statement does not name {name} with the sha256 {sha256}"))
}

/// Whether Rekor signed `entry`, the entry is of this very signature, and its time is inside the
/// certificate's validity.
// shortcut: trusts Rekor's signed promise, not the Merkle proof or the SCT; add them if Rekor v1's key is ever in doubt.
fn logged(
    entry: &Entry,
    leaf: &Leaf,
    der: &[u8],
    payload: &[u8],
    signature: &str,
) -> Checked<bool> {
    let body = base64(&entry.canonicalized_body)?;
    let time: i64 = entry.integrated_time.parse().map_err(|_| "a bad time")?;
    let index: u64 = entry.log_index.parse().map_err(|_| "a bad index")?;
    // `base64` accepted the body and the log id, so neither needs escaping.
    let promised = format!(
        r#"{{"body":"{}","integratedTime":{time},"logID":"{}","logIndex":{index}}}"#,
        entry.canonicalized_body,
        to_hex(&base64(&entry.log_id.key_id)?)
    );
    let promise = base64(&entry.inclusion_promise.signed_entry_timestamp)?;
    if signed(false, &hex(REKOR), promised.as_bytes(), &promise).is_err() {
        return Ok(false);
    }
    let body: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    let spec = &body["spec"];
    let Some([one]) = spec["signatures"].as_array().map(Vec::as_slice) else {
        return Ok(false);
    };
    let pem = base64(one["verifier"].as_str().unwrap_or_default())?;
    let armored: String = String::from_utf8_lossy(&pem)
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    Ok(body["kind"] == "dsse"
        && body["apiVersion"] == "0.0.1"
        && spec["payloadHash"]["algorithm"] == "sha256"
        && spec["payloadHash"]["value"] == to_hex(&Sha256::digest(payload))
        && one["signature"] == signature
        && base64(&armored)? == der
        && (leaf.from..=leaf.until).contains(&time))
}

/// Whether `key` made `signature` over `message`: P-384 with SHA-384 when `wide`, else P-256 with
/// SHA-256.
fn signed(wide: bool, key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), ()> {
    let (curve, hash) = match wide {
        true => (ECDSA_P384, ECDSA_SHA384),
        false => (ECDSA_P256, ECDSA_SHA256),
    };
    default_provider()
        .signature_verification_algorithms
        .all
        .iter()
        .find(|algorithm| {
            algorithm.public_key_alg_id() == curve && algorithm.signature_alg_id() == hash
        })
        .ok_or(())?
        .verify_signature(key, message, signature)
        .map_err(|_| ())
}

/// What Priory reads of a Fulcio certificate.
struct Leaf<'a> {
    signed: &'a [u8],
    signature: &'a [u8],
    key: &'a [u8],
    identity: &'a [u8],
    issuer: &'a [u8],
    from: i64,
    until: i64,
}

impl<'a> Leaf<'a> {
    /// Reads `der`, refusing anything past what Fulcio issues to a workflow: another algorithm, an
    /// extension that appears twice, a critical one it does not know, a byte left over.
    fn parse(der: &'a [u8]) -> Option<Leaf<'a>> {
        let mut outer = Der(der);
        let mut certificate = Der(outer.take(0x30)?);
        outer.end()?;
        let whole = certificate.0;
        let mut body = Der(certificate.take(0x30)?);
        let signed = &whole[..whole.len() - certificate.0.len()];
        let algorithm = certificate.take(0x30)?;
        let signature = certificate.take(0x03)?.strip_prefix(&[0])?;
        certificate.end()?;
        // Version 3.
        if body.take(0xa0)? != [0x02, 0x01, 0x02] {
            return None;
        }
        body.take(0x02)?;
        if body.take(0x30)? != algorithm || algorithm != ECDSA_SHA384.as_ref() {
            return None;
        }
        body.take(0x30)?;
        let mut validity = Der(body.take(0x30)?);
        let (from, until) = (validity.time()?, validity.time()?);
        validity.end()?;
        body.take(0x30)?;
        let mut info = Der(body.take(0x30)?);
        if info.take(0x30)? != ECDSA_P256.as_ref() {
            return None;
        }
        let key = info.take(0x03)?.strip_prefix(&[0])?;
        info.end()?;
        let mut wrapped = Der(body.take(0xa3)?);
        body.end()?;
        let mut extensions = Der(wrapped.take(0x30)?);
        wrapped.end()?;
        let (mut identity, mut issuer, mut usage, mut purpose) = (None, None, None, None);
        let mut seen: Vec<&[u8]> = Vec::new();
        while !extensions.0.is_empty() {
            let mut extension = Der(extensions.take(0x30)?);
            let id = extension.take(0x06)?;
            let critical = match extension.0.first() {
                Some(0x01) => extension.take(0x01)? == [0xff],
                _ => false,
            };
            let value = extension.take(0x04)?;
            extension.end()?;
            if seen.contains(&id) {
                return None;
            }
            seen.push(id);
            let mut inner = Der(value);
            match id {
                KEY_USAGE => usage = Some(inner.take(0x03)?),
                EXTENDED_KEY_USAGE => purpose = Some(inner.all()),
                ALT_NAME => {
                    let mut names = Der(inner.take(0x30)?);
                    identity = Some(names.take(0x86)?);
                    names.end()?;
                }
                OIDC_ISSUER => issuer = Some(inner.take(0x0c)?),
                _ if critical => return None,
                _ => {
                    inner.all();
                }
            }
            inner.end()?;
        }
        // Digital signature alone: the first bit, with 7 unused ones after it.
        if usage? != [0x07, 0x80] || purpose? != CODE_SIGNING {
            return None;
        }
        Some(Leaf {
            signed,
            signature,
            key,
            identity: identity?,
            issuer: issuer?,
            from,
            until,
        })
    }
}

/// The DER bytes left to read.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    /// The content of the next value, which must have `tag`.
    fn take(&mut self, tag: u8) -> Option<&'a [u8]> {
        let (&found, rest) = self.0.split_first()?;
        let (&first, mut rest) = rest.split_first()?;
        if found != tag {
            return None;
        }
        let length = match first {
            0..=0x7f => usize::from(first),
            0x81 | 0x82 => {
                let (bytes, after) = rest.split_at_checked(usize::from(first & 0x0f))?;
                rest = after;
                let length = bytes
                    .iter()
                    .fold(0, |length, byte| length << 8 | usize::from(*byte));
                // DER has one form for each length: the shortest.
                let least = if first == 0x81 { 0x80 } else { 0x100 };
                (length >= least).then_some(length)?
            }
            _ => return None,
        };
        let (content, rest) = rest.split_at_checked(length)?;
        self.0 = rest;
        Some(content)
    }

    fn all(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    /// A UTCTime, which holds until 2049, as seconds since the epoch.
    fn time(&mut self) -> Option<i64> {
        let text = std::str::from_utf8(self.take(0x17)?).ok()?;
        let digits = text.strip_suffix('Z')?;
        if digits.len() != 12 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let at = jiff::civil::DateTime::strptime("%Y%m%d%H%M%S", format!("20{digits}")).ok()?;
        let at = at.to_zoned(jiff::tz::TimeZone::UTC).ok()?;
        Some(at.timestamp().as_second())
    }

    fn end(&self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

fn base64(text: &str) -> Checked<Vec<u8>> {
    use base64ct::Encoding as _;
    base64ct::Base64::decode_vec(text).map_err(|_| "it holds bad base64".to_owned())
}

/// The bytes of a pinned key.
fn hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |byte: u8| (byte as char).to_digit(16).expect("a pinned key is hex") as u8;
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect()
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// What GitHub answers for the x86_64 binary of v1.3.3, with no token.
    const ANSWER: &str = include_str!("provenance.v1.3.3.json");
    const REPOSITORY: &str = "maxence-lefebvre/priory";
    const NAME: &str = "priory-x86_64-unknown-linux-musl";
    const DIGEST: &str = "184e3435e4a7f29bfa3ad8b2f26715f33ea170d0da67aa6a24c44a5e557d4c45";

    fn check(answer: &str) -> Checked<()> {
        verify(answer.as_bytes(), REPOSITORY, "1.3.3", NAME, DIGEST)
    }

    /// The answer with `edit` made to its one bundle.
    fn edited(edit: impl FnOnce(&mut Value)) -> Checked<()> {
        let mut answer: Value = serde_json::from_str(ANSWER).unwrap();
        edit(&mut answer["attestations"][0]["bundle"]);
        check(&answer.to_string())
    }

    /// Flips one bit of the base64 bytes at `value`, at `at` from the end.
    fn flip(value: &mut Value, at: usize) {
        use base64ct::Encoding as _;
        let mut bytes = base64(value.as_str().unwrap()).unwrap();
        let index = bytes.len() - 1 - at;
        bytes[index] ^= 1;
        *value = base64ct::Base64::encode_string(&bytes).into();
    }

    #[test]
    fn a_release_s_own_attestation_verifies() {
        assert_eq!(check(ANSWER), Ok(()));
    }

    #[test]
    fn another_tag_repository_name_or_digest_is_refused() {
        let cases = [
            (REPOSITORY, "1.3.2", NAME, DIGEST, "another workflow or tag"),
            (
                "evil/priory",
                "1.3.3",
                NAME,
                DIGEST,
                "another workflow or tag",
            ),
            (REPOSITORY, "1.3.3", "priory", DIGEST, "does not name"),
            (REPOSITORY, "1.3.3", NAME, &DIGEST[1..], "does not name"),
        ];
        for (repository, version, name, digest, why) in cases {
            let refused = verify(ANSWER.as_bytes(), repository, version, name, digest);
            assert!(refused.as_ref().unwrap_err().contains(why), "{refused:?}");
        }
    }

    #[test]
    fn a_changed_payload_signature_certificate_or_log_entry_is_refused() {
        let refused = [
            (
                edited(|bundle| flip(&mut bundle["dsseEnvelope"]["payload"], 9)),
                "payload's signature",
            ),
            (
                edited(|bundle| flip(&mut bundle["dsseEnvelope"]["signatures"][0]["sig"], 0)),
                "payload's signature",
            ),
            (
                edited(|bundle| {
                    flip(
                        &mut bundle["verificationMaterial"]["certificate"]["rawBytes"],
                        0,
                    );
                }),
                "Fulcio did not sign",
            ),
            (
                edited(|bundle| {
                    flip(
                        &mut bundle["verificationMaterial"]["certificate"]["rawBytes"],
                        400,
                    );
                }),
                "certificate",
            ),
            (
                edited(|bundle| bundle["dsseEnvelope"]["payloadType"] = "text/plain".into()),
                "its payload is text/plain",
            ),
            (
                edited(|bundle| bundle["dsseEnvelope"]["signatures"] = Value::Array(vec![])),
                "one signature",
            ),
        ];
        for (refused, why) in refused {
            assert!(refused.as_ref().unwrap_err().contains(why), "{refused:?}");
        }
        for pointer in [
            "/inclusionPromise/signedEntryTimestamp",
            "/canonicalizedBody",
            "/logId/keyId",
        ] {
            let refused = edited(|bundle| {
                let entry = &mut bundle["verificationMaterial"]["tlogEntries"][0];
                flip(entry.pointer_mut(pointer).unwrap(), 3);
            });
            assert!(refused.unwrap_err().contains("no Rekor entry"), "{pointer}");
        }
        for key in ["integratedTime", "logIndex"] {
            let refused = edited(|bundle| {
                bundle["verificationMaterial"]["tlogEntries"][0][key] = "1795530000".into();
            });
            assert!(refused.unwrap_err().contains("no Rekor entry"), "{key}");
        }
    }

    #[test]
    fn no_attestation_is_refused() {
        let refused = check(r#"{"attestations":[]}"#);
        assert_eq!(refused, Err("GitHub holds no attestation of it".into()));
    }

    fn certificate() -> Vec<u8> {
        let answer: Value = serde_json::from_str(ANSWER).unwrap();
        let material = &answer["attestations"][0]["bundle"]["verificationMaterial"];
        base64(material["certificate"]["rawBytes"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn a_certificate_cut_short_or_with_a_byte_more_does_not_parse() {
        let mut der = certificate();
        assert!(Leaf::parse(&der).is_some());
        for end in 0..der.len() {
            assert!(Leaf::parse(&der[..end]).is_none(), "{end}");
        }
        der.push(0);
        assert!(Leaf::parse(&der).is_none());
    }

    #[test]
    fn the_time_of_the_log_entry_must_be_inside_the_certificate_s_validity() {
        let answer: Attestations = serde_json::from_str(ANSWER).unwrap();
        let bundle = &answer.attestations[0].bundle;
        let der = certificate();
        let payload = base64(&bundle.dsse_envelope.payload).unwrap();
        let signature = &bundle.dsse_envelope.signatures[0].sig;
        let entry = &bundle.verification_material.tlog_entries[0];
        let mut leaf = Leaf::parse(&der).unwrap();
        assert_eq!(logged(entry, &leaf, &der, &payload, signature), Ok(true));
        leaf.until = leaf.from - 1;
        assert_eq!(logged(entry, &leaf, &der, &payload, signature), Ok(false));
    }

    proptest! {
        #[test]
        fn random_and_mutated_certificates_never_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..300),
            at in any::<prop::sample::Index>(),
            byte in any::<u8>(),
        ) {
            let _ = Leaf::parse(&bytes);
            let mut der = certificate();
            let at = at.index(der.len());
            der[at] = byte;
            let _ = Leaf::parse(&der);
        }

        #[test]
        fn random_and_mutated_answers_are_refused(
            text in ".{0,200}",
            at in any::<prop::sample::Index>(),
            byte in any::<u8>(),
        ) {
            prop_assert!(check(&text).is_err());
            let mut answer = ANSWER.as_bytes().to_vec();
            let at = at.index(answer.len());
            answer[at] = byte;
            let _ = verify(&answer, REPOSITORY, "1.3.3", NAME, DIGEST);
        }
    }
}
