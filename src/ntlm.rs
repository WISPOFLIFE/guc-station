//! Minimal NTLMv2 client (MS-NLMP). GUC's IIS servers only offer
//! Negotiate/NTLM, so every request goes through this handshake.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use hmac::{Hmac, Mac};
use md4::{Digest, Md4};
use md5::Md5;
use rand::RngCore;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacMd5 = Hmac<Md5>;

const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";

const NEGOTIATE_UNICODE: u32 = 0x0000_0001;
const REQUEST_TARGET: u32 = 0x0000_0004;
const NEGOTIATE_NTLM: u32 = 0x0000_0200;
const ALWAYS_SIGN: u32 = 0x0000_8000;
const EXTENDED_SESSION_SECURITY: u32 = 0x0008_0000;
const NEGOTIATE_TARGET_INFO: u32 = 0x0080_0000;
const NEGOTIATE_128: u32 = 0x2000_0000;
const NEGOTIATE_56: u32 = 0x8000_0000;

const FLAGS: u32 = NEGOTIATE_UNICODE
    | REQUEST_TARGET
    | NEGOTIATE_NTLM
    | ALWAYS_SIGN
    | EXTENDED_SESSION_SECURITY
    | NEGOTIATE_TARGET_INFO
    | NEGOTIATE_128
    | NEGOTIATE_56;

/// AV pair id carrying the server's FILETIME in the challenge's target info.
const MSV_AV_TIMESTAMP: u16 = 7;

pub struct Challenge {
    server_challenge: [u8; 8],
    target_name: String,
    target_info: Vec<u8>,
}

impl Challenge {
    pub fn target_name(&self) -> &str {
        &self.target_name
    }
}

/// Type 1 message, base64-encoded for the `Authorization: NTLM ...` header.
pub fn negotiate_message() -> String {
    let mut msg = Vec::with_capacity(32);
    msg.extend_from_slice(SIGNATURE);
    msg.extend_from_slice(&1u32.to_le_bytes());
    msg.extend_from_slice(&FLAGS.to_le_bytes());
    // Empty domain and workstation security buffers.
    msg.extend_from_slice(&[0u8; 16]);
    B64.encode(msg)
}

/// Parses the Type 2 message from a `WWW-Authenticate: NTLM <b64>` header value.
pub fn parse_challenge(header: &str) -> Option<Challenge> {
    let b64 = header.trim().strip_prefix("NTLM ")?.trim();
    let data = B64.decode(b64).ok()?;
    if data.len() < 32 || &data[..8] != SIGNATURE || u32_at(&data, 8)? != 2 {
        return None;
    }
    let target_name = read_buffer(&data, 12).map(utf16_decode).unwrap_or_default();
    let mut server_challenge = [0u8; 8];
    server_challenge.copy_from_slice(data.get(24..32)?);
    let target_info = if data.len() >= 48 {
        read_buffer(&data, 40).map(|b| b.to_vec()).unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(Challenge { server_challenge, target_name, target_info })
}

/// Type 3 message answering `challenge` with NTLMv2 responses.
/// When `domain` is empty the server's own target name is used.
pub fn authenticate_message(challenge: &Challenge, domain: &str, user: &str, password: &str) -> String {
    let domain = if domain.is_empty() { challenge.target_name.as_str() } else { domain };

    let nt_hash = Md4::digest(utf16(password));
    let ntowf_v2 = hmac_md5(&nt_hash, &[&utf16(&(user.to_uppercase() + domain))]);

    let mut client_challenge = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut client_challenge);
    let server_time = find_av_pair(&challenge.target_info, MSV_AV_TIMESTAMP);
    let timestamp = match server_time {
        Some(t) if t.len() == 8 => t.to_vec(),
        _ => filetime_now().to_le_bytes().to_vec(),
    };

    let mut blob = Vec::new();
    blob.extend_from_slice(&[1, 1, 0, 0, 0, 0, 0, 0]);
    blob.extend_from_slice(&timestamp);
    blob.extend_from_slice(&client_challenge);
    blob.extend_from_slice(&[0u8; 4]);
    blob.extend_from_slice(&challenge.target_info);
    blob.extend_from_slice(&[0u8; 4]);

    let nt_proof = hmac_md5(&ntowf_v2, &[&challenge.server_challenge, &blob]);
    let mut nt_response = nt_proof.to_vec();
    nt_response.extend_from_slice(&blob);

    // When the server sent a timestamp, LMv2 must be zeroed (MS-NLMP 3.1.5.1.2).
    let lm_response = if server_time.is_some() {
        vec![0u8; 24]
    } else {
        let mut lm = hmac_md5(&ntowf_v2, &[&challenge.server_challenge, &client_challenge]).to_vec();
        lm.extend_from_slice(&client_challenge);
        lm
    };

    let payloads: [Vec<u8>; 6] = [
        lm_response,
        nt_response,
        utf16(domain),
        utf16(user),
        utf16("GUCSTATION"),
        Vec::new(), // session key
    ];

    let header_len = 64u32;
    let mut msg = Vec::new();
    msg.extend_from_slice(SIGNATURE);
    msg.extend_from_slice(&3u32.to_le_bytes());
    let mut offset = header_len;
    for p in &payloads {
        let len = p.len() as u16;
        msg.extend_from_slice(&len.to_le_bytes());
        msg.extend_from_slice(&len.to_le_bytes());
        msg.extend_from_slice(&offset.to_le_bytes());
        offset += p.len() as u32;
    }
    msg.extend_from_slice(&FLAGS.to_le_bytes());
    for p in &payloads {
        msg.extend_from_slice(p);
    }
    B64.encode(msg)
}

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let mut mac = HmacMd5::new_from_slice(key).expect("HMAC accepts any key length");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

fn utf16_decode(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn read_buffer(d: &[u8], at: usize) -> Option<&[u8]> {
    let len = u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?) as usize;
    let off = u32_at(d, at + 4)? as usize;
    d.get(off..off + len)
}

fn find_av_pair(info: &[u8], id: u16) -> Option<&[u8]> {
    let mut i = 0;
    while i + 4 <= info.len() {
        let av_id = u16::from_le_bytes([info[i], info[i + 1]]);
        let len = u16::from_le_bytes([info[i + 2], info[i + 3]]) as usize;
        if av_id == 0 {
            return None;
        }
        let value = info.get(i + 4..i + 4 + len)?;
        if av_id == id {
            return Some(value);
        }
        i += 4 + len;
    }
    None
}

fn filetime_now() -> u64 {
    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (since_epoch.as_secs() + 11_644_473_600) * 10_000_000 + u64::from(since_epoch.subsec_nanos() / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_has_signature_and_type() {
        let raw = B64.decode(negotiate_message()).unwrap();
        assert_eq!(&raw[..8], SIGNATURE);
        assert_eq!(u32_at(&raw, 8), Some(1));
    }

    #[test]
    fn ntowf_v2_matches_spec_vector() {
        // MS-NLMP 4.2.4.1.1: user "User", domain "Domain", password "Password".
        let nt_hash = Md4::digest(utf16("Password"));
        let v2 = hmac_md5(&nt_hash, &[&utf16(&("User".to_uppercase() + "Domain"))]);
        let hex: String = v2.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "0c868a403bfd7a93a3001ef22ef02e3f");
    }
}
