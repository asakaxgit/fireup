//! A LevelDB log reader written from the spec, independent of `src/`.
//!
//! The fixtures under `tests/fixtures/` are only useful as evidence if something
//! other than fireup agrees they are well formed. This module is that something:
//! it re-derives every masked CRC32C and re-assembles every fragment using only
//! `leveldb/db/log_format.h`, so a fireup failure against a fixture these
//! functions accept is a fireup finding rather than a fixture artifact.
//!
//! Shared by `tests/real_export_validation.rs` and
//! `tests/export_fidelity_tests.rs` via `#[path = "support/export_log_reader.rs"]`.
//! Keeping one copy matters: two hand-written spec readers would drift, and the
//! whole point is that this one is not derived from the code under test.

#![allow(dead_code)] // Each test binary uses a different subset.

use std::path::{Path, PathBuf};

pub const BLOCK_SIZE: usize = 32768;
pub const HEADER_SIZE: usize = 7;

pub const RECORD_FULL: u8 = 1;
pub const RECORD_FIRST: u8 = 2;
pub const RECORD_MIDDLE: u8 = 3;
pub const RECORD_LAST: u8 = 4;

/// CRC32C (Castagnoli), the polynomial LevelDB actually uses.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x82F6_3B78
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// LevelDB stores masked CRCs so a stored checksum is never a checksum of real data.
pub fn mask_crc(crc: u32) -> u32 {
    ((crc >> 15) | (crc << 17)).wrapping_add(0xA282_EAD8)
}

/// One physical record header, kept so tests can assert on the framing itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fragment {
    pub record_type: u8,
    pub payload_len: usize,
    /// Zero-based index of the 32 KiB block this fragment sits in.
    pub block: usize,
    /// Byte offset of the header within its block.
    pub offset: usize,
}

/// A fully unframed log file: logical records plus the physical framing.
#[derive(Debug, Default)]
pub struct LogFile {
    pub records: Vec<Vec<u8>>,
    pub fragments: Vec<Fragment>,
}

impl LogFile {
    pub fn record_types(&self) -> Vec<u8> {
        self.fragments.iter().map(|f| f.record_type).collect()
    }

    pub fn has_record_type(&self, record_type: u8) -> bool {
        self.fragments.iter().any(|f| f.record_type == record_type)
    }
}

/// Unframe a LevelDB log file, verifying every record checksum.
///
/// Panics with the file and offset on any spec violation, so a broken fixture
/// names itself instead of surfacing as a confusing downstream assertion.
pub fn read_log(path: &Path) -> LogFile {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = LogFile::default();
    let mut pending: Vec<u8> = Vec::new();

    for (block_index, block) in raw.chunks(BLOCK_SIZE).enumerate() {
        let mut offset = 0usize;
        while offset + HEADER_SIZE <= block.len() {
            let header = &block[offset..offset + HEADER_SIZE];
            let checksum = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let length = u16::from_le_bytes([header[4], header[5]]) as usize;
            let record_type = header[6];
            if checksum == 0 && length == 0 && record_type == 0 {
                break; // trailing padding
            }

            let start = offset + HEADER_SIZE;
            assert!(
                start + length <= block.len(),
                "{}: record at block {block_index}+{offset} runs past its block",
                path.display()
            );
            let body = &block[start..start + length];

            let mut hashed = Vec::with_capacity(length + 1);
            hashed.push(record_type);
            hashed.extend_from_slice(body);
            assert_eq!(
                mask_crc(crc32c(&hashed)),
                checksum,
                "{}: masked CRC32C mismatch at block {block_index}+{offset}",
                path.display()
            );

            out.fragments.push(Fragment {
                record_type,
                payload_len: length,
                block: block_index,
                offset,
            });

            match record_type {
                RECORD_FULL => {
                    assert!(pending.is_empty(), "FULL record inside an open fragment");
                    out.records.push(body.to_vec());
                }
                RECORD_FIRST => {
                    assert!(pending.is_empty(), "FIRST record inside an open fragment");
                    pending = body.to_vec();
                }
                RECORD_MIDDLE | RECORD_LAST => {
                    assert!(!pending.is_empty(), "fragment without a preceding FIRST");
                    pending.extend_from_slice(body);
                    if record_type == RECORD_LAST {
                        out.records.push(std::mem::take(&mut pending));
                    }
                }
                other => panic!("invalid LevelDB record type {other}"),
            }

            offset = start + length;
        }
    }

    assert!(pending.is_empty(), "{}: ends mid-fragment", path.display());
    out
}

/// Logical records of a log file, for tests that do not care about framing.
pub fn read_log_records(path: &Path) -> Vec<Vec<u8>> {
    read_log(path).records
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// `<tree>/all_namespaces/kind_<kind>/output-<shard>`
pub fn kind_shard(tree: &str, kind: &str, shard: u32) -> PathBuf {
    partition_shard(tree, "all_namespaces", kind, shard)
}

/// `<tree>/all_namespaces/kind_<kind>/output-0`
pub fn kind_output(tree: &str, kind: &str) -> PathBuf {
    kind_shard(tree, kind, 0)
}

/// `<tree>/<partition>/kind_<kind>/output-<shard>`, where `partition` is
/// `all_namespaces` or `namespace_<ns>`.
pub fn partition_shard(tree: &str, partition: &str, kind: &str, shard: u32) -> PathBuf {
    fixtures_dir()
        .join(tree)
        .join(partition)
        .join(format!("kind_{kind}"))
        .join(format!("output-{shard}"))
}
