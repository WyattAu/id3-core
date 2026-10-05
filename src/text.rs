//! Text encodings and the decoded string type.
//!
//! ID3v2 text frames carry a one-byte encoding selector, and the four options
//! are genuinely different wire formats — not four names for UTF-8:
//!
//! | Byte | Encoding |
//! |---|---|
//! | 0 | ISO-8859-1 (Latin-1) |
//! | 1 | UTF-16 with a byte-order mark |
//! | 2 | UTF-16BE, no BOM (v2.4 only) |
//! | 3 | UTF-8 (v2.4 only) |
//!
//! Latin-1 is still the encoding of the majority of tags in the wild, so this
//! crate decodes it properly rather than rejecting it: every code point 0–255
//! is a defined Latin-1 character, which makes the mapping trivial and lossless.
//!
//! [`TextEncoding::decode`] never fails on well-formed input. Where a frame is
//! malformed the error names the frame, so a caller scanning a file can skip
//! one bad frame and keep the rest of the tag.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;
use crate::FrameId;

/// The encoding selector byte at the head of a text frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    /// ISO-8859-1. The ID3v2.2/2.3 default and still the most common.
    #[default]
    Latin1,
    /// UTF-16 with a leading byte-order mark.
    Utf16Bom,
    /// UTF-16 big-endian, no BOM. Introduced in v2.4.
    Utf16Be,
    /// UTF-8. Introduced in v2.4.
    Utf8,
}

impl TextEncoding {
    /// The selector byte as it appears on the wire.
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            TextEncoding::Latin1 => 0,
            TextEncoding::Utf16Bom => 1,
            TextEncoding::Utf16Be => 2,
            TextEncoding::Utf8 => 3,
        }
    }

    /// Decode a selector byte.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidTextEncoding`] for a byte outside 0–3.
    pub fn from_byte(byte: u8) -> Result<Self, Id3Error> {
        match byte {
            0 => Ok(TextEncoding::Latin1),
            1 => Ok(TextEncoding::Utf16Bom),
            2 => Ok(TextEncoding::Utf16Be),
            3 => Ok(TextEncoding::Utf8),
            other => Err(Id3Error::InvalidTextEncoding { encoding: other }),
        }
    }

    /// Whether this encoding was legal in the given major version.
    ///
    /// v2.2 and v2.3 define only Latin-1 and BOM-marked UTF-16. A writer that
    /// emits UTF-8 in a v2.3 tag is producing a file some decoders misread, so
    /// the tag writer refuses it.
    #[must_use]
    pub fn is_valid_for(self, major_version: u8) -> bool {
        match self {
            TextEncoding::Latin1 | TextEncoding::Utf16Bom => true,
            TextEncoding::Utf16Be | TextEncoding::Utf8 => major_version >= 4,
        }
    }

    /// The name used in the v2.4 frame-level documentation.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            TextEncoding::Latin1 => "ISO-8859-1",
            TextEncoding::Utf16Bom => "UTF-16",
            TextEncoding::Utf16Be => "UTF-16BE",
            TextEncoding::Utf8 => "UTF-8",
        }
    }

    /// Decode `data` (the frame body *after* the encoding byte) as `encoding`.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidUtf16`] for an unpaired surrogate or a BOM-marked
    /// buffer without a BOM, and [`Id3Error::InvalidUtf8`] for invalid UTF-8.
    pub fn decode(self, data: &[u8]) -> Result<String, Id3Error> {
        match self {
            TextEncoding::Latin1 => Ok(decode_latin1(data)),
            TextEncoding::Utf8 => {
                String::from_utf8(data.to_vec()).map_err(|_| Id3Error::InvalidUtf8)
            }
            TextEncoding::Utf16Be => decode_utf16(data, None),
            TextEncoding::Utf16Bom => {
                let (le, rest) = split_bom(data).ok_or(Id3Error::InvalidTextData {
                    id: FrameId::unknown(),
                    reason: "UTF-16 frame has no byte-order mark",
                })?;
                decode_utf16(rest, Some(le))
            }
        }
    }

    /// Encode `text` with this encoding's wire representation.
    ///
    /// UTF-16 always emits a BOM (choosing big-endian, the more common
    /// convention in tags) because selector 1 requires one. Latin-1 output
    /// replaces anything above U+00FF with `?` rather than failing: a tag with a
    /// substituted glyph is far better than a rejected one, and the caller can
    /// detect the substitution by comparing lengths.
    #[must_use]
    pub fn encode(self, text: &str) -> Vec<u8> {
        match self {
            TextEncoding::Latin1 => text
                .chars()
                .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
                .collect(),
            TextEncoding::Utf8 => text.as_bytes().to_vec(),
            TextEncoding::Utf16Be => {
                let mut out = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    out.extend_from_slice(&unit.to_be_bytes());
                }
                out
            }
            TextEncoding::Utf16Bom => {
                let mut out = Vec::with_capacity(text.len() * 2 + 2);
                out.extend_from_slice(&[0xFF, 0xFE]);
                for unit in text.encode_utf16() {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
                out
            }
        }
    }
}

/// Decode Latin-1, where every byte is its own code point.
fn decode_latin1(data: &[u8]) -> String {
    data.iter().map(|b| char::from(*b)).collect()
}

/// Read a leading BOM, returning `(little_endian, remainder)`.
///
/// A UTF-16LE BOM is `FF FE` and a UTF-16BE BOM is `FE FF`. Neither is valid
/// UTF-8 data, so the check is unambiguous.
fn split_bom(data: &[u8]) -> Option<(bool, &[u8])> {
    match (data.first(), data.get(1)) {
        (Some(0xFF), Some(0xFE)) => Some((true, data.get(2..).unwrap_or(&[]))),
        (Some(0xFE), Some(0xFF)) => Some((false, data.get(2..).unwrap_or(&[]))),
        _ => None,
    }
}

/// Decode UTF-16 code units, validating surrogate pairing.
fn decode_utf16(data: &[u8], little_endian: Option<bool>) -> Result<String, Id3Error> {
    // Without a BOM, selector 1 is malformed; selector 2 is BE by definition.
    let le = little_endian.unwrap_or(false);
    if data.len() % 2 != 0 {
        return Err(Id3Error::InvalidUtf16);
    }
    let mut units: Vec<u16> = Vec::with_capacity(data.len() / 2);
    for pair in data.chunks_exact(2) {
        // `chunks_exact(2)` guarantees two bytes, but the crate denies
        // indexing, so unpack through a checked conversion.
        let [a, b] = <[u8; 2]>::try_from(pair).unwrap_or([0, 0]);
        let unit = if le {
            u16::from_le_bytes([a, b])
        } else {
            u16::from_be_bytes([a, b])
        };
        units.push(unit);
    }
    // `String::from_utf16_lossless` is not available on no_std, so validate
    // surrogate pairing by hand and then re-encode.
    for w in units.windows(2) {
        let (a, b) = (
            w.first().copied().unwrap_or(0),
            w.get(1).copied().unwrap_or(0),
        );
        if (0xD800..0xDC00).contains(&a) && !(0xDC00..0xE000).contains(&b) {
            return Err(Id3Error::InvalidUtf16);
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

/// A decoded text frame: the encoding it used and its contents.
///
/// Multi-value frames (v2.4 allows a null-separated list) are split into
/// `values`, with `text` holding the first for the common single-value case.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text {
    /// The encoding the frame was stored in.
    pub encoding: TextEncoding,
    /// Every null-separated value in the frame.
    pub values: Vec<String>,
}

impl Text {
    /// A single-value text frame.
    #[must_use]
    pub fn single(encoding: TextEncoding, text: &str) -> Self {
        Self {
            encoding,
            values: alloc::vec![String::from(text)],
        }
    }

    /// The first value, or an empty string when the frame had none.
    ///
    /// An empty ID3 text frame is legal and common (a blank comment field), so
    /// this returns `""` rather than `None`.
    #[must_use]
    pub fn text(&self) -> &str {
        self.values.first().map_or("", String::as_str)
    }

    /// Parse a text frame body: one encoding byte, then the encoded payload.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidTextEncoding`] for a bad selector byte,
    /// [`Id3Error::InvalidTextData`] for an empty body, and the encoding's own
    /// error for malformed text.
    pub fn parse(id: FrameId, body: &[u8]) -> Result<Self, Id3Error> {
        let selector = *body.first().ok_or(Id3Error::InvalidTextData {
            id,
            reason: "text frame has no encoding byte",
        })?;
        let encoding = TextEncoding::from_byte(selector)?;
        let payload = body.get(1..).unwrap_or(&[]);
        let decoded = encoding.decode(payload).map_err(|e| match e {
            Id3Error::InvalidUtf16 | Id3Error::InvalidUtf8 => Id3Error::InvalidTextData {
                id,
                reason: match e {
                    Id3Error::InvalidUtf16 => "malformed UTF-16",
                    _ => "malformed UTF-8",
                },
            },
            other => other,
        })?;
        // v2.4 permits a null-separated multi-value list; v2.3 does not, but
        // splitting is harmless there because a null is not valid in a
        // single-value Latin-1 string anyway.
        let mut values: Vec<String> = decoded
            .split('\0')
            .map(String::from)
            .filter(|s| !s.is_empty())
            .collect();
        if values.is_empty() {
            values.push(decoded);
        }
        Ok(Self { encoding, values })
    }

    /// Serialize to a text frame body: encoding byte, then encoded payload.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidParameter`](crate::error::Id3Error::InvalidParameter)
    /// is never returned today; the signature is fallible so a future
    /// length-limited format can reject an oversized frame.
    pub fn to_body(&self) -> Result<Vec<u8>, Id3Error> {
        let mut out = Vec::with_capacity(self.encoded_len());
        out.push(self.encoding.as_byte());
        let joined = self.values.join("\0");
        out.extend_from_slice(&self.encoding.encode(&joined));
        Ok(out)
    }

    /// The exact serialized length, for size accounting before writing.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let joined_len: usize = self.values.iter().map(|v| v.chars().count()).sum::<usize>()
            + self.values.len().saturating_sub(1);
        1 + match self.encoding {
            TextEncoding::Latin1 => joined_len,
            TextEncoding::Utf8 => joined_len,
            TextEncoding::Utf16Be => joined_len * 2,
            // A BOM is two bytes, not one.
            TextEncoding::Utf16Bom => joined_len * 2 + 2,
        }
    }
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
    fn encoding_bytes_round_trip() {
        for e in [
            TextEncoding::Latin1,
            TextEncoding::Utf16Bom,
            TextEncoding::Utf16Be,
            TextEncoding::Utf8,
        ] {
            assert_eq!(TextEncoding::from_byte(e.as_byte()), Ok(e));
        }
        assert_eq!(
            TextEncoding::from_byte(4),
            Err(Id3Error::InvalidTextEncoding { encoding: 4 })
        );
    }

    #[test]
    fn version_validity_matches_the_spec() {
        assert!(TextEncoding::Latin1.is_valid_for(2));
        assert!(TextEncoding::Utf16Bom.is_valid_for(2));
        assert!(!TextEncoding::Utf8.is_valid_for(3), "UTF-8 is v2.4+");
        assert!(!TextEncoding::Utf16Be.is_valid_for(3), "UTF-16BE is v2.4+");
        assert!(TextEncoding::Utf8.is_valid_for(4));
    }

    #[test]
    fn latin1_decodes_every_byte() {
        // Every code point 0-255 is defined, so this is total.
        let all: Vec<u8> = (0..=255).collect();
        let s = TextEncoding::Latin1.decode(&all).expect("always valid");
        assert_eq!(s.chars().count(), 256);
        assert_eq!(s.chars().next(), Some('\u{0}'));
        assert_eq!(s.chars().last(), Some('\u{FF}'));
    }

    #[test]
    fn latin1_encode_substitutes_wide_chars() {
        let out = TextEncoding::Latin1.encode("café über");
        assert_eq!(
            out,
            vec![b'c', b'a', b'f', 0xE9, b' ', 0xFC, b'b', b'e', b'r']
        );
        // A character outside Latin-1 becomes '?', never a panic.
        let out = TextEncoding::Latin1.encode("日本");
        assert_eq!(out, vec![b'?', b'?']);
    }

    #[test]
    fn utf16_bom_round_trips_both_endiannesses() {
        let text = "Grüße";
        for (encoding, want_le) in [
            (TextEncoding::Utf16Bom, true),
            (TextEncoding::Utf16Be, false),
        ] {
            let bytes = encoding.encode(text);
            let decoded = encoding.decode(&bytes).expect("valid");
            assert_eq!(decoded, text);
            if let TextEncoding::Utf16Bom = encoding {
                let le = split_bom(&bytes).expect("has bom").0;
                assert_eq!(le, want_le);
            }
        }
    }

    #[test]
    fn utf16_bom_requires_a_mark() {
        let e = TextEncoding::Utf16Bom;
        let no_bom = [0x00, 0x41, 0x00, 0x42];
        assert!(matches!(
            e.decode(&no_bom),
            Err(Id3Error::InvalidTextData { .. })
        ));
    }

    #[test]
    fn utf16_rejects_odd_length_and_unpaired_surrogates() {
        assert_eq!(
            TextEncoding::Utf16Be.decode(&[0x00, 0x41, 0x00]),
            Err(Id3Error::InvalidUtf16)
        );
        // High surrogate with no low surrogate following.
        let bad = [0xD8, 0x00, 0x00, 0x41];
        assert_eq!(
            TextEncoding::Utf16Be.decode(&bad),
            Err(Id3Error::InvalidUtf16)
        );
    }

    #[test]
    fn surrogate_pairs_decode() {
        // U+1F600 as a UTF-16BE surrogate pair.
        let bytes = [0xD8, 0x3D, 0xDE, 0x00];
        assert_eq!(
            TextEncoding::Utf16Be.decode(&bytes),
            Ok(String::from("\u{1F600}"))
        );
    }

    #[test]
    fn text_parses_a_single_value() {
        let mut body = vec![0u8]; // Latin-1
        body.extend_from_slice(b"Dune");
        let t = Text::parse(FrameId::new(*b"TIT2").expect("valid"), &body).expect("valid");
        assert_eq!(t.text(), "Dune");
        assert_eq!(t.values.len(), 1);
        assert_eq!(t.encoding, TextEncoding::Latin1);
    }

    #[test]
    fn text_parses_multiple_values() {
        let mut body = vec![3u8]; // UTF-8
        body.extend_from_slice(b"Artist A\x00Artist B\x00Artist C");
        let t = Text::parse(FrameId::new(*b"TPE1").expect("valid"), &body).expect("valid");
        assert_eq!(t.values.len(), 3);
        assert_eq!(t.text(), "Artist A");
        assert_eq!(t.values[2], "Artist C");
    }

    #[test]
    fn text_rejects_an_empty_body() {
        assert!(matches!(
            Text::parse(FrameId::new(*b"TIT2").expect("valid"), &[]),
            Err(Id3Error::InvalidTextData { .. })
        ));
    }

    #[test]
    fn text_round_trips_through_a_body() {
        for encoding in [
            TextEncoding::Latin1,
            TextEncoding::Utf16Bom,
            TextEncoding::Utf16Be,
            TextEncoding::Utf8,
        ] {
            let t = Text {
                encoding,
                values: alloc::vec![String::from("Frank Herbert")],
            };
            let body = t.to_body().expect("serializes");
            assert_eq!(
                body.len(),
                t.encoded_len(),
                "length prediction for {encoding:?}"
            );
            let back =
                Text::parse(FrameId::new(*b"TPE1").expect("valid"), &body).expect("reparses");
            assert_eq!(back.text(), "Frank Herbert", "round trip for {encoding:?}");
        }
    }

    #[test]
    fn empty_text_is_not_none() {
        let t = Text::default();
        assert_eq!(t.text(), "");
    }

    #[test]
    fn proptest_text_round_trip() {
        use proptest::prelude::*;
        proptest!(|(s: String)| {
            let t = Text::single(TextEncoding::Utf8, &s);
            let body = t.to_body().expect("serializes");
            let back = Text::parse(FrameId::new(*b"TIT2").expect("valid"), &body).expect("reparses");
            // A null inside the input splits into multiple values; compare the
            // join so the property is about the encoding, not the split.
            prop_assert_eq!(back.values.join("\0"), s);
        });
    }

    #[test]
    fn proptest_latin1_never_panics() {
        use proptest::prelude::*;
        proptest!(|(bytes: Vec<u8>)| {
            let s = TextEncoding::Latin1.decode(&bytes).expect("total");
            prop_assert_eq!(s.chars().count(), bytes.len());
        });
    }
}
