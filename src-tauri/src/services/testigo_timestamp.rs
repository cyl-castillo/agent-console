//! RFC 3161 trusted timestamp of a proof packet's signature (Testigo V2-B,
//! spec §2.5). The token imprints sha256 of the raw DSSE signature bytes —
//! the CAdES signature-time-stamp construction — proving the signing act
//! (and therefore everything signed) existed no later than the TSA's genTime.
//!
//! Scope, honestly stated: we BUILD the request and do a minimal structural
//! check of the response (status granted, our digest echoed inside). We do
//! not validate the token's CMS signature — that is the receiver's job with
//! real tooling (`openssl ts -verify`), and the verifier says so instead of
//! pretending. Requesting a token sends the TSA only a signature hash, never
//! ledger content.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// DER TimeStampReq for a sha256 imprint:
/// SEQUENCE { version 1, MessageImprint { sha256 AlgId, OCTET STRING digest },
/// certReq TRUE } — certReq so the token embeds the TSA cert chain and the
/// packet stays verifiable without fetching anything.
pub fn request_der(digest: &[u8; 32]) -> Vec<u8> {
    // AlgorithmIdentifier: OID 2.16.840.1.101.3.4.2.1 (sha256) + NULL params.
    const SHA256_ALG_ID: [u8; 15] = [
        0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00,
    ];
    let mut out = Vec::with_capacity(59);
    out.extend_from_slice(&[0x30, 0x39]); // TimeStampReq, 57 content bytes
    out.extend_from_slice(&[0x02, 0x01, 0x01]); // version 1
    out.extend_from_slice(&[0x30, 0x31]); // MessageImprint, 49 content bytes
    out.extend_from_slice(&SHA256_ALG_ID);
    out.extend_from_slice(&[0x04, 0x20]); // OCTET STRING, 32 bytes
    out.extend_from_slice(digest);
    out.extend_from_slice(&[0x01, 0x01, 0xff]); // certReq TRUE
    out
}

/// One DER TLV header: (content_start, content_len) or None if malformed.
fn der_header(buf: &[u8], at: usize) -> Option<(usize, usize)> {
    let first = *buf.get(at + 1)?;
    if first < 0x80 {
        return Some((at + 2, first as usize));
    }
    let n = (first & 0x7f) as usize;
    if n == 0 || n > 4 {
        return None;
    }
    let mut len = 0usize;
    for i in 0..n {
        len = (len << 8) | *buf.get(at + 2 + i)? as usize;
    }
    Some((at + 2 + n, len))
}

/// Minimal structural check of a TimeStampResp: PKIStatus granted (0) or
/// grantedWithMods (1), a token present after the status, and our exact
/// digest bytes echoed inside it (the token's messageImprint appears
/// literally in the TSTInfo DER). NOT a cryptographic verification.
pub fn check_response(resp: &[u8], digest: &[u8; 32]) -> Result<(), String> {
    if resp.first() != Some(&0x30) {
        return Err("response is not a DER SEQUENCE".into());
    }
    let (status_at, _) = der_header(resp, 0).ok_or("malformed response length")?;
    if resp.get(status_at) != Some(&0x30) {
        return Err("missing PKIStatusInfo".into());
    }
    let (int_at, status_len) = der_header(resp, status_at).ok_or("malformed PKIStatusInfo")?;
    if resp.get(int_at) != Some(&0x02) {
        return Err("missing PKIStatus".into());
    }
    let (val_at, val_len) = der_header(resp, int_at).ok_or("malformed PKIStatus")?;
    let status = match resp.get(val_at..val_at + val_len) {
        Some([b]) => *b as u64,
        _ => return Err("unexpected PKIStatus width".into()),
    };
    if status > 1 {
        return Err(format!("TSA refused the request (PKIStatus {status})"));
    }
    let token_at = int_at + status_len; // end of the PKIStatusInfo TLV
    if token_at >= resp.len() {
        return Err("granted status but no token in response".into());
    }
    if !resp.windows(digest.len()).any(|w| w == digest) {
        return Err("token does not echo the requested digest".into());
    }
    Ok(())
}

/// POST the request to the TSA on a dedicated OS thread with its own tiny
/// tokio runtime. The dance exists for one constraint: this is called from
/// the synchronous export path, which may itself run on a tokio worker —
/// where both `block_on` and nested runtimes panic. A fresh thread has no
/// async context, so this works from anywhere.
fn post_tsq(tsa_url: &str, body: Vec<u8>) -> AppResult<Vec<u8>> {
    let url = tsa_url.to_string();
    std::thread::spawn(move || -> AppResult<Vec<u8>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AppError::Other(format!("tsa runtime: {e}")))?;
        rt.block_on(async {
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| AppError::Other(format!("tsa client: {e}")))?
                .post(&url)
                .header("Content-Type", "application/timestamp-query")
                .body(body)
                .send()
                .await
                .map_err(|e| AppError::Other(format!("tsa request: {e}")))?;
            if !resp.status().is_success() {
                return Err(AppError::Other(format!("tsa http {}", resp.status())));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::Other(format!("tsa body: {e}")))?;
            Ok(bytes.to_vec())
        })
    })
    .join()
    .map_err(|_| AppError::Other("tsa thread panicked".into()))?
}

/// Obtain the packet's `timestamp` member: hash the signature, query the TSA,
/// structurally check the grant, and wrap the raw TimeStampResp. Failure is
/// an error, not a silent omission — the user opted into the timestamp, so a
/// packet quietly missing it would misrepresent what they asked to produce.
pub fn obtain(tsa_url: &str, signature: &[u8]) -> AppResult<Value> {
    let mut h = Sha256::new();
    h.update(signature);
    let digest: [u8; 32] = h.finalize().into();
    let resp = post_tsq(tsa_url, request_der(&digest))?;
    check_response(&resp, &digest)
        .map_err(|e| AppError::Other(format!("tsa response rejected: {e}")))?;
    Ok(json!({
        "type": "rfc3161",
        "tsaUrl": tsa_url,
        "hashAlg": "sha256",
        "messageImprint": digest.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "token": B64.encode(&resp),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden request bytes: must match what `openssl ts -query -sha256
    /// -no_nonce -cert` produces for the same digest (captured once).
    #[test]
    fn request_der_matches_openssl() {
        let digest: [u8; 32] = (0u8..32).collect::<Vec<_>>().try_into().unwrap();
        let der = request_der(&digest);
        assert_eq!(der.len(), 59);
        let hex: String = der.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "30390201013031300d060960864801650304020105000420\
             000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
             0101ff"
        );
    }

    /// Happy path against a real freetsa.org response captured as a fixture.
    #[test]
    fn accepts_real_granted_response() {
        let resp = include_bytes!("../../tests/fixtures/freetsa-granted.tsr");
        let imprint: [u8; 32] = <[u8; 32]>::try_from(
            hex_to_bytes("1f5e0a0a09534e1d81debb2be79d1dd6e418f656d2f9a700bc93fcc99618e218")
                .as_slice(),
        )
        .unwrap();
        assert!(check_response(resp, &imprint).is_ok());
        // Same token, different digest → the echo check must fail.
        let other = [0xabu8; 32];
        assert!(check_response(resp, &other).is_err());
    }

    /// A rejection (PKIStatus 2) with no token must be refused even though
    /// it is well-formed DER.
    #[test]
    fn rejects_denied_status() {
        // SEQUENCE { SEQUENCE { INTEGER 2 } }
        let resp = [0x30u8, 0x05, 0x30, 0x03, 0x02, 0x01, 0x02];
        let digest = [0u8; 32];
        let err = check_response(&resp, &digest).unwrap_err();
        assert!(err.contains("PKIStatus 2"), "{err}");
    }

    /// Granted status but nothing after PKIStatusInfo → no token → refuse.
    #[test]
    fn rejects_granted_without_token() {
        let resp = [0x30u8, 0x05, 0x30, 0x03, 0x02, 0x01, 0x00];
        let digest = [0u8; 32];
        let err = check_response(&resp, &digest).unwrap_err();
        assert!(err.contains("no token"), "{err}");
    }

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
