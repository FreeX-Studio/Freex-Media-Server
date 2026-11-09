/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! Passwords, tokens, brute-force protection and two-factor sign-in (TOTP, RFC 6238).

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Milliseconds since 1970, the time format of the API.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Argon2id hash in PHC format.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).expect("argon2 hashing").to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// 256 random bits, URL-safe: session tokens, challenges.
pub fn token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Short random id for database rows (96 bits, hex).
pub fn id() -> String {
    let mut b = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn sha256(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

// ------------------------------------------------------------------ brute-force protection

/// Slows down guessing: an address that failed 20 times within 10 minutes has to wait for the
/// rest of the window; an account waits twice as long after every failure beyond the fifth, up
/// to 15 minutes. Known and unknown accounts are treated the same.
#[derive(Default)]
pub struct LoginGuard {
    state: Mutex<GuardState>,
}

#[derive(Default)]
struct GuardState {
    by_ip: HashMap<String, Counter>,
    by_account: HashMap<String, Counter>,
}

#[derive(Clone, Copy)]
struct Counter {
    count: u32,
    first: Instant,
    last: Instant,
}

const IP_LIMIT: u32 = 20;
const IP_WINDOW: Duration = Duration::from_secs(10 * 60);
const FREE_ATTEMPTS: u32 = 5;
const MAX_DELAY: Duration = Duration::from_secs(15 * 60);

impl LoginGuard {
    /// Seconds to wait before the next attempt, 0 if none.
    pub fn retry_after(&self, ip: &str, account: &str) -> u64 {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        let mut wait = Duration::ZERO;
        if let Some(c) = s.by_ip.get(ip).copied() {
            let end = c.first + IP_WINDOW;
            if now > end {
                s.by_ip.remove(ip);
            } else if c.count >= IP_LIMIT {
                wait = end - now;
            }
        }
        if let Some(a) = s.by_account.get(&key(account))
            && a.count >= FREE_ATTEMPTS
        {
            let shift = (a.count - FREE_ATTEMPTS + 1).min(20);
            let delay = Duration::from_millis(1000u64 << shift).min(MAX_DELAY);
            let until = a.last + delay;
            if until > now {
                wait = wait.max(until - now);
            }
        }
        if wait.is_zero() { 0 } else { wait.as_secs().max(1) + u64::from(wait.subsec_nanos() > 0) }
    }

    pub fn failure(&self, ip: &str, account: &str) {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        let count = |map: &mut HashMap<String, Counter>, k: String| {
            map.entry(k)
                .and_modify(|c| {
                    c.count += 1;
                    c.last = now;
                })
                .or_insert(Counter { count: 1, first: now, last: now });
        };
        count(&mut s.by_ip, ip.to_string());
        count(&mut s.by_account, key(account));
        if s.by_ip.len() > 10_000 {
            let cutoff = now - Duration::from_secs(3600);
            s.by_ip.retain(|_, c| c.last > cutoff);
            s.by_account.retain(|_, c| c.last > cutoff);
        }
    }

    pub fn success(&self, ip: &str, account: &str) {
        let mut s = self.state.lock().unwrap();
        s.by_account.remove(&key(account));
        if let Some(c) = s.by_ip.get_mut(ip) {
            c.count = c.count.saturating_sub(1);
        }
    }
}

fn key(account: &str) -> String {
    account.trim().to_lowercase()
}

// ------------------------------------------------------------------ TOTP

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// A new 160-bit secret in base32, as authenticator apps expect it.
pub fn totp_secret() -> String {
    let mut b = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut b);
    base32_encode(&b)
}

/// `otpauth://` URI for the QR code.
pub fn totp_uri(issuer: &str, account: &str, secret: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20");
    format!("otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30", enc(issuer), enc(account), enc(issuer))
}

/// Checks a 6-digit code against the current, previous and next 30-second step. Returns the
/// step on success; steps up to `last_step` are rejected, so a code cannot be used twice.
pub fn totp_verify(secret: &str, code: &str, last_step: i64) -> Option<i64> {
    let code = code.trim().replace(' ', "");
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let key = base32_decode(secret)?;
    let now = now_ms() / 1000 / 30;
    (now - 1..=now + 1).find(|&step| step > last_step && totp_code(&key, step) == code)
}

fn totp_code(key: &[u8], step: i64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key).expect("any key length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let o = (h[19] & 0x0f) as usize;
    let v = (u32::from(h[o] & 0x7f) << 24) | (u32::from(h[o + 1]) << 16) | (u32::from(h[o + 2]) << 8) | u32::from(h[o + 3]);
    format!("{:06}", v % 1_000_000)
}

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for &b in data {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(BASE32[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for c in s.trim_end_matches('=').bytes() {
        let v = BASE32.iter().position(|&x| x == c.to_ascii_uppercase())? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buffer >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// Ten one-time recovery codes: shown once as `abcde-fghij`, stored as hashes.
pub fn recovery_codes() -> (Vec<String>, Vec<String>) {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let mut plain = Vec::new();
    let mut hashes = Vec::new();
    for _ in 0..10 {
        let code: String = (0..10).map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char).collect();
        plain.push(format!("{}-{}", &code[..5], &code[5..]));
        hashes.push(sha256(&code));
    }
    (plain, hashes)
}

/// The hash of a recovery code as typed (spaces, dashes and case do not matter), if it looks like one.
pub fn recovery_code_hash(input: &str) -> Option<String> {
    let normalized: String = input.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_lowercase();
    (normalized.len() == 10 && normalized.chars().all(|c| c.is_ascii_alphanumeric())).then(|| sha256(&normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        let h = hash_password("correct horse");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn totp_matches_rfc6238() {
        // RFC 6238 test key "12345678901234567890", T = 59 s -> step 1 -> 94287082 (8 digits)
        let key = b"12345678901234567890";
        assert_eq!(totp_code(key, 1), "287082");
        assert_eq!(base32_decode(&base32_encode(key)).unwrap(), key);
    }

    #[test]
    fn totp_rejects_replay() {
        let secret = totp_secret();
        let key = base32_decode(&secret).unwrap();
        let step = now_ms() / 1000 / 30;
        let code = totp_code(&key, step);
        assert_eq!(totp_verify(&secret, &code, 0), Some(step));
        assert_eq!(totp_verify(&secret, &code, step), None);
    }

    #[test]
    fn guard_waits_after_free_attempts() {
        let g = LoginGuard::default();
        for _ in 0..FREE_ATTEMPTS {
            assert_eq!(g.retry_after("1.2.3.4", "Alice"), 0);
            g.failure("1.2.3.4", "alice");
        }
        assert!(g.retry_after("5.6.7.8", "ALICE") > 0);
        g.success("1.2.3.4", "alice");
        assert_eq!(g.retry_after("5.6.7.8", "alice"), 0);
    }

    #[test]
    fn recovery_codes_round_trip() {
        let (plain, hashes) = recovery_codes();
        assert_eq!(recovery_code_hash(&plain[0].to_uppercase()).unwrap(), hashes[0]);
        assert!(recovery_code_hash("short").is_none());
    }
}
