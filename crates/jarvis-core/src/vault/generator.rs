//! Password generation with the system CSPRNG.
//!
//! The generator never uses `Math.random` or any non-cryptographic source: bytes
//! come from `getrandom`, and index selection uses rejection sampling so no
//! character is more likely than another. `generate_password_from` exists so the
//! sampling can be tested deterministically without weakening production.

use serde::{Deserialize, Serialize};

use super::model::VaultError;

/// Lowest accepted length.
pub const MIN_LENGTH: usize = 8;
/// Highest accepted length.
pub const MAX_LENGTH: usize = 128;
/// Recommended default length.
pub const DEFAULT_LENGTH: usize = 20;

const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.?/";
/// Characters that are easy to confuse in many fonts.
const SIMILAR: &[u8] = b"il1IL|Lo0O";

/// Which characters a generated password may contain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct PasswordPolicy {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Drop characters that look alike (`i`, `l`, `1`, `L`, `o`, `0`, `O`, `|`).
    pub exclude_similar: bool,
    /// Guarantee at least one character from every selected category.
    pub require_each_category: bool,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            length: DEFAULT_LENGTH,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            exclude_similar: false,
            require_each_category: true,
        }
    }
}

/// The character classes selected by a policy.
fn categories(policy: &PasswordPolicy) -> Vec<&'static [u8]> {
    let mut selected: Vec<&'static [u8]> = Vec::new();
    if policy.lowercase {
        selected.push(LOWERCASE);
    }
    if policy.uppercase {
        selected.push(UPPERCASE);
    }
    if policy.digits {
        selected.push(DIGITS);
    }
    if policy.symbols {
        selected.push(SYMBOLS);
    }
    selected
}

fn filter_similar(alphabet: &[u8], exclude_similar: bool) -> Vec<u8> {
    alphabet
        .iter()
        .copied()
        .filter(|byte| !exclude_similar || !SIMILAR.contains(byte))
        .collect()
}

/// Maps a random byte onto `0..limit` without modulo bias.
///
/// Bytes at or above the largest multiple of `limit` below 256 are rejected and
/// drawn again, which is what makes every index equally likely instead of
/// favouring the first `256 % limit` values.
pub fn uniform_index_from(limit: usize, next_byte: &mut impl FnMut() -> u8) -> usize {
    debug_assert!((1..=256).contains(&limit), "limit must fit in one byte");
    let bound = 256 - (256 % limit);
    loop {
        let byte = next_byte() as usize;
        if byte < bound {
            return byte % limit;
        }
    }
}

fn system_byte() -> u8 {
    let mut buffer = [0u8; 1];
    // `getrandom` only fails when the OS has no CSPRNG at all; in that case a
    // predictable password must never be produced, so the generator panics
    // rather than falling back to a weaker source.
    getrandom::fill(&mut buffer).expect("the operating system must provide randomness");
    buffer[0]
}

/// Generates a password with the system CSPRNG.
pub fn generate_password(policy: &PasswordPolicy) -> Result<String, VaultError> {
    let mut next_byte = system_byte;
    generate_password_from(policy, &mut next_byte)
}

/// Generates a password from an explicit byte source. Used by tests.
pub fn generate_password_from(
    policy: &PasswordPolicy,
    next_byte: &mut impl FnMut() -> u8,
) -> Result<String, VaultError> {
    if policy.length < MIN_LENGTH || policy.length > MAX_LENGTH {
        return Err(VaultError::InvalidPasswordPolicy);
    }
    let selected = categories(policy);
    if selected.is_empty() {
        return Err(VaultError::InvalidPasswordPolicy);
    }
    let filtered: Vec<Vec<u8>> = selected
        .iter()
        .map(|alphabet| filter_similar(alphabet, policy.exclude_similar))
        .collect();
    if filtered.iter().any(|alphabet| alphabet.is_empty()) {
        // Only possible if a whole category were nothing but similar characters.
        return Err(VaultError::InvalidPasswordPolicy);
    }
    if policy.require_each_category && policy.length < filtered.len() {
        return Err(VaultError::InvalidPasswordPolicy);
    }

    let mut alphabet: Vec<u8> = Vec::new();
    for category in &filtered {
        alphabet.extend_from_slice(category);
    }

    let mut characters: Vec<u8> = Vec::with_capacity(policy.length);
    if policy.require_each_category {
        for category in &filtered {
            let index = uniform_index_from(category.len(), next_byte);
            characters.push(category[index]);
        }
    }
    while characters.len() < policy.length {
        let index = uniform_index_from(alphabet.len(), next_byte);
        characters.push(alphabet[index]);
    }

    // Fisher-Yates with the same unbiased sampling, so the guaranteed characters
    // are not always in the first positions.
    for position in (1..characters.len()).rev() {
        let swap_with = uniform_index_from(position + 1, next_byte);
        characters.swap(position, swap_with);
    }

    let password = String::from_utf8(characters).map_err(|_| VaultError::InvalidPasswordPolicy)?;
    Ok(password)
}

/// Rough strength hint for the interface, in bits of entropy.
pub fn estimate_entropy_bits(policy: &PasswordPolicy) -> f64 {
    let selected = categories(policy);
    let size: usize = selected
        .iter()
        .map(|alphabet| filter_similar(alphabet, policy.exclude_similar).len())
        .sum();
    if size <= 1 {
        return 0.0;
    }
    (policy.length as f64) * (size as f64).log2()
}
