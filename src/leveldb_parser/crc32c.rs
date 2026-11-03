//! CRC32C (Castagnoli) plus LevelDB's checksum masking.
//!
//! LevelDB log records are checksummed with CRC32C — the reflected Castagnoli
//! polynomial `0x82F63B78` — over `type_byte || payload`, and the result is then
//! *masked* so that a stored checksum is never the checksum of real data
//! (`leveldb/util/crc32c.h`). The `crc32fast` crate implements IEEE CRC-32
//! (`0xEDB88320`) and cannot produce these values, so the table below is the
//! parser's own implementation. It is a table-driven, byte-at-a-time loop; swap
//! in a hardware-accelerated implementation only if profiling shows the
//! checksum is the bottleneck (parsing dominates in practice).

const CASTAGNOLI_POLY: u32 = 0x82F6_3B78;

/// LevelDB's checksum mask delta (`kMaskDelta`).
const MASK_DELTA: u32 = 0xA282_EAD8;

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ CASTAGNOLI_POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
}

static TABLE: [u32; 256] = build_table();

/// Continue a CRC32C over more data. `previous` must be a finalized CRC
/// (`crc32c` output), which makes the function chainable without exposing the
/// internal inverted state.
pub fn extend(previous: u32, data: &[u8]) -> u32 {
    let mut crc = !previous;
    for &byte in data {
        crc = TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// CRC32C of a single buffer.
pub fn crc32c(data: &[u8]) -> u32 {
    extend(0, data)
}

/// Apply LevelDB's checksum mask: rotate right 15 bits, then add `kMaskDelta`.
pub fn mask(crc: u32) -> u32 {
    ((crc >> 15) | (crc << 17)).wrapping_add(MASK_DELTA)
}

/// Reverse [`mask`]. Useful when reporting a stored checksum as a raw CRC.
pub fn unmask(masked: u32) -> u32 {
    let rotated = masked.wrapping_sub(MASK_DELTA);
    (rotated >> 17) | (rotated << 15)
}

/// The masked CRC32C a LevelDB log record header stores for `record_type || data`.
pub fn masked_record_checksum(record_type: u8, data: &[u8]) -> u32 {
    mask(extend(crc32c(&[record_type]), data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_crc32c_check_vectors() {
        // Standard CRC-32C check value for the ASCII string "123456789".
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0x0000_0000);
        // CRC32C differs from the IEEE CRC-32 that crc32fast computes.
        assert_ne!(crc32c(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn extend_matches_a_single_pass() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let split = extend(crc32c(&data[..10]), &data[10..]);
        assert_eq!(split, crc32c(data));
    }

    #[test]
    fn mask_round_trips() {
        for crc in [0u32, 1, 0xE306_9283, u32::MAX] {
            assert_eq!(unmask(mask(crc)), crc);
        }
        // Masking must move the value, otherwise a checksum could be the
        // checksum of real data.
        assert_ne!(mask(0xE306_9283), 0xE306_9283);
    }

    #[test]
    fn record_checksum_includes_the_type_byte() {
        let payload = b"record payload";
        assert_eq!(
            masked_record_checksum(1, payload),
            mask(crc32c(&[&[1u8][..], payload].concat()))
        );
        assert_ne!(
            masked_record_checksum(1, payload),
            masked_record_checksum(2, payload)
        );
    }
}
