//! Standard (RFC 4648) base64 encoding, used to carry raw export bytes through
//! JSON without corrupting them.
//!
//! `protobuf_entity.rs` still has an equivalent private copy; fold the two
//! together once the `datastore.v1` value mapping settles.

const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Why a base64 string could not be decoded. Decoding is strict: the local
/// store's `bytesValue` payloads round-trip through this, so quietly accepting
/// a malformed string would mean handing a client bytes it never wrote.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("base64 length {0} is not a multiple of 4")]
    Length(usize),
    #[error("base64 contains the invalid character {0:?}")]
    InvalidCharacter(char),
    #[error("base64 padding is misplaced")]
    Padding,
}

/// Encode bytes as padded base64.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Decode padded standard base64 back to bytes.
pub fn decode(text: &str) -> Result<Vec<u8>, DecodeError> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(DecodeError::Length(bytes.len()));
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, quad) in bytes.chunks(4).enumerate() {
        let last_quad = index == bytes.len() / 4 - 1;
        let mut sextets = [0u32; 4];
        let mut payload_len = 4;

        for (position, byte) in quad.iter().enumerate() {
            if *byte == b'=' {
                // Padding is only legal in the final quad, and only in the
                // last two positions.
                if !last_quad || position < 2 {
                    return Err(DecodeError::Padding);
                }
                payload_len = payload_len.min(position);
                continue;
            }
            // A character after padding started would decode to data that the
            // encoder could never have produced.
            if payload_len < 4 {
                return Err(DecodeError::Padding);
            }
            let sextet = TABLE
                .iter()
                .position(|candidate| candidate == byte)
                .ok_or(DecodeError::InvalidCharacter(*byte as char))?;
            sextets[position] = sextet as u32;
        }

        let triple = (sextets[0] << 18) | (sextets[1] << 12) | (sextets[2] << 6) | sextets[3];
        out.push(((triple >> 16) & 0xFF) as u8);
        if payload_len > 2 {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if payload_len > 3 {
            out.push((triple & 0xFF) as u8);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc4648_test_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn encodes_non_utf8_bytes() {
        assert_eq!(encode(&[0xFF, 0xFE, 0xFD]), "//79");
        assert_eq!(encode(&[0x00, 0x00, 0x00]), "AAAA");
    }

    #[test]
    fn decode_matches_the_rfc4648_test_vectors() {
        for original in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
        ] {
            assert_eq!(decode(&encode(original)).as_deref(), Ok(original));
        }
    }

    #[test]
    fn decodes_every_byte_value() {
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&all_bytes)), Ok(all_bytes));
    }

    #[test]
    fn rejects_malformed_input_rather_than_returning_wrong_bytes() {
        assert_eq!(decode("Zg="), Err(DecodeError::Length(3)));
        assert_eq!(decode("Zg=="), Ok(b"f".to_vec()));
        assert!(matches!(decode("Z!=="), Err(DecodeError::InvalidCharacter('!'))));
        assert_eq!(decode("=Zm8"), Err(DecodeError::Padding));
        assert_eq!(decode("Zm=8"), Err(DecodeError::Padding));
        // Padding in a non-final quad would silently shorten the output.
        assert_eq!(decode("Zg==Zg=="), Err(DecodeError::Padding));
    }
}
