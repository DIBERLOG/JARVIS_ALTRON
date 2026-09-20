//! Tiny, deterministic artifacts for the setup tests.
//!
//! Nothing here is ever fetched from the network, and nothing here is a real
//! model or a real executable: a GGUF header, a PE header, and a zip archive are
//! all that the validators look at, so a few kilobytes prove the same behaviour
//! that five gigabytes would.

use std::io::Write;

/// A GGUF file of exactly `size` bytes with the given metadata.
pub fn gguf(size: u64, architecture: &str, file_type: u32) -> Vec<u8> {
    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&2u64.to_le_bytes());
    push_string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    push_string(&mut bytes, architecture);
    push_string(&mut bytes, "general.file_type");
    bytes.extend_from_slice(&4u32.to_le_bytes());
    bytes.extend_from_slice(&file_type.to_le_bytes());
    assert!(
        size as usize >= bytes.len(),
        "the fixture must be at least as large as its header"
    );
    bytes.resize(size as usize, 0);
    bytes
}

/// The metadata of the pinned model, in a small file.
pub fn pinned_gguf(size: u64) -> Vec<u8> {
    gguf(size, "qwen3", 15)
}

/// A minimal PE image with the given machine tag.
///
/// The body is filled with `bytes` rather than zeroes so the image behaves like
/// a real executable in a zip: a run of zeroes compresses to almost nothing and
/// would be refused by the decompression-bomb guard for the wrong reason.
pub fn pe_stub(machine: u16) -> Vec<u8> {
    let mut bytes = self::bytes(0x400, machine as u8 | 1);
    bytes[0..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    bytes
}

/// An x86-64 `llama-server.exe` stand-in.
pub fn pe_x64() -> Vec<u8> {
    pe_stub(0x8664)
}

/// Bytes that are not a PE image at all.
pub fn not_a_pe() -> Vec<u8> {
    b"this is not an executable".to_vec()
}

/// A zip archive with exactly the given entries.
pub fn zip_archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, contents) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer.into_inner()
}

/// Reproducible filler bytes that do not repeat and do not compress.
///
/// `xorshift64*` has a full period, so a few hundred kilobytes of it are not a
/// repeating pattern: a cheap generator with a short period would be compressed
/// away by the archive writer and would then be refused by the
/// decompression-bomb guard, hiding the behaviour the fixture is meant to test.
pub fn bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ (seed as u64).wrapping_mul(0x2545_f491_4f6c_dd1d);
    // A few warm-up rounds so a small seed is not visible in the first bytes.
    for _ in 0..8 {
        state = xorshift(state);
    }
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        state = xorshift(state);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn xorshift(mut state: u64) -> u64 {
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// Lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}
