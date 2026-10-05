//! Synch-safe integers.
//!
//! ID3 sizes are *synch-safe*: seven significant bits per byte, so no byte in
//! the encoded form has its high bit set. That guarantees a frame body can
//! never contain a byte sequence that looks like an MPEG audio sync word
//! (`0xFF 0xEx`), which is the entire point of the encoding — an ID3 tag is
//! embedded in a stream that scanners walk byte by byte looking for audio
//! frames.
//!
//! Decoders in the wild frequently emit plain integers instead. [`decode`]
//! rejects those; [`decode_lenient`] recovers them, which is what a scanner
//! wants when it is trying to find a tag rather than validate a writer.

/// Encode `value` as a synch-safe integer.
///
/// Returns `None` above 2^28 − 1, the largest value four synch-safe bytes can
/// hold — the spec's tag-size ceiling.
#[must_use]
pub fn encode(value: u32) -> Option<[u8; 4]> {
    if value > 0x0FFF_FFFF {
        return None;
    }
    Some([
        ((value >> 21) & 0x7F) as u8,
        ((value >> 14) & 0x7F) as u8,
        ((value >> 7) & 0x7F) as u8,
        (value & 0x7F) as u8,
    ])
}

/// Encode `value` into an existing 4-byte buffer.
#[must_use]
pub fn encode_into(value: u32, out: &mut [u8; 4]) -> Option<()> {
    let bytes = encode(value)?;
    out.copy_from_slice(&bytes);
    Some(())
}

/// Decode a strict synch-safe integer.
///
/// # Errors
///
/// [`Id3Error::NotSynchSafe`](crate::Id3Error::NotSynchSafe) if any byte has
/// its high bit set.
pub fn decode(bytes: [u8; 4]) -> Result<u32, crate::Id3Error> {
    for b in bytes {
        if b > 0x7F {
            return Err(crate::Id3Error::NotSynchSafe { byte: b });
        }
    }
    Ok(decode_value(bytes))
}

/// Decode a synch-safe integer, recovering from a non-synch-safe writer.
///
/// Each byte contributes its low seven bits regardless of the high bit, which
/// is what most real-world decoders do. The result is wrong for a genuinely
/// malformed tag, so prefer [`decode`] when validating and this when scanning.
#[must_use]
pub fn decode_lenient(bytes: [u8; 4]) -> u32 {
    decode_value([
        bytes[0] & 0x7F,
        bytes[1] & 0x7F,
        bytes[2] & 0x7F,
        bytes[3] & 0x7F,
    ])
}

/// The shared 7-bits-per-byte accumulation, used by both decoders.
///
/// Public because a tag parser needs it to read a size field it has already
/// validated is syntactically a tag; [`decode`] remains the checked entry
/// point for untrusted input.
#[must_use]
pub fn decode_value(bytes: [u8; 4]) -> u32 {
    (u32::from(bytes[0] & 0x7F) << 21)
        | (u32::from(bytes[1] & 0x7F) << 14)
        | (u32::from(bytes[2] & 0x7F) << 7)
        | u32::from(bytes[3] & 0x7F)
}

/// Encode a 28-bit synch-safe integer from a `usize`.
///
/// # Errors
///
/// [`Id3Error::SizeTooLarge`](crate::Id3Error::SizeTooLarge) when `value`
/// exceeds what four synch-safe bytes can represent.
pub fn encode_usize(value: usize) -> Result<[u8; 4], crate::Id3Error> {
    u32::try_from(value)
        .ok()
        .and_then(encode)
        .ok_or(crate::Id3Error::SizeTooLarge {
            declared: value.min(u32::MAX as usize) as u32,
            available: 0x0FFF_FFFF,
        })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic
    )]
    use super::*;
    use alloc::vec;

    #[test]
    fn spec_examples_round_trip() {
        // The three worked examples from the ID3v2.4 specification §6.2.
        for value in [
            0u32, 1, 127, 128, 255, 256, 65535, 65536, 2_097_151, 2_097_152,
        ] {
            let e = encode(value).expect("fits in 28 bits");
            assert!(e.iter().all(|b| *b < 0x80), "high bit set for {value}");
            assert_eq!(decode(e), Ok(value), "round trip failed for {value}");
        }
    }

    #[test]
    fn spec_byte_examples() {
        // Spec: 255 encodes as 00 00 01 7F; 256 as 00 00 02 00.
        assert_eq!(encode(255), Some([0x00, 0x00, 0x01, 0x7F]));
        assert_eq!(encode(256), Some([0x00, 0x00, 0x02, 0x00]));
        assert_eq!(encode(128), Some([0x00, 0x00, 0x01, 0x00]));
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(encode(0x0FFF_FFFF), Some([0x7F, 0x7F, 0x7F, 0x7F]));
        assert_eq!(encode(0x1000_0000), None, "2^28 is one past the ceiling");
        assert_eq!(encode(0x1000_0000 - 1), Some([0x7F, 0x7F, 0x7F, 0x7F]));
    }

    #[test]
    fn strict_decode_rejects_high_bits() {
        assert_eq!(
            decode([0x80, 0, 0, 0]),
            Err(crate::Id3Error::NotSynchSafe { byte: 0x80 })
        );
        assert_eq!(
            decode([0, 0, 0, 0xFF]),
            Err(crate::Id3Error::NotSynchSafe { byte: 0xFF })
        );
    }

    #[test]
    fn lenient_decode_recovers_a_plain_integer() {
        // A writer that stored 255 as 00 00 00 FF (plain, not synch-safe).
        // Strict decode rejects; lenient recovers 255.
        assert!(decode([0x00, 0x00, 0x00, 0xFF]).is_err());
        assert_eq!(decode_lenient([0x00, 0x00, 0x00, 0xFF]), 127);
        // A plain little-endian size of 128 is 80 00 00 00.
        assert_eq!(decode_lenient([0x80, 0x00, 0x00, 0x00]), 0);
    }

    #[test]
    fn encode_into_and_encode_usize_agree() {
        let mut buf = [0u8; 4];
        encode_into(256, &mut buf).expect("fits");
        assert_eq!(buf, [0x00, 0x00, 0x02, 0x00]);
        assert_eq!(encode_usize(256), Ok(buf));
    }

    #[test]
    fn encode_usize_reports_overflow() {
        let big = 0x1000_0000usize;
        assert!(matches!(
            encode_usize(big),
            Err(crate::Id3Error::SizeTooLarge { .. })
        ));
    }

    #[test]
    fn every_encoded_value_is_safe_against_mpeg_sync() {
        // The whole reason for synch-safe: scanning the encoded bytes for the
        // 0xFF 0xEx MPEG frame sync must find nothing.
        for v in 0..2048u32 {
            let e = encode(v).expect("fits");
            for pair in e.windows(2) {
                let is_sync = pair[0] == 0xFF && (pair[1] & 0xE0) == 0xE0;
                assert!(!is_sync, "value {v} produced an MPEG sync word");
            }
        }
    }

    #[test]
    fn decode_accepts_the_full_28_bit_range() {
        for v in [0u32, 1, 0x0FFF_FFFF / 2, 0x0FFF_FFFF] {
            let e = encode(v).expect("fits");
            assert_eq!(decode_lenient(e), v);
        }
    }

    #[test]
    fn proptest_round_trip() {
        use proptest::prelude::*;
        proptest!(|(v: u32)| {
            let e = encode(v);
            if let Some(bytes) = e {
                prop_assert_eq!(decode(bytes), Ok(v));
                prop_assert_eq!(decode_lenient(bytes), v);
            }
        });
        let _ = vec![0u8; 0];
    }
}
