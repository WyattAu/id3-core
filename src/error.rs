//! The crate-wide error type.
//!
//! Every fallible parse returns [`Id3Error`]. Parsing is deliberately
//! total-ish: a malformed or truncated frame produces an error the caller can
//! skip past, never a panic — an unknown frame id is not an error at all, just
//! a [`Frame::Unknown`](crate::Frame::Unknown) the caller can ignore.

use core::fmt;

use crate::frame::FrameId;

/// Errors returned by tag and frame parsers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Id3Error {
    /// The input did not begin with the `ID3` identifier.
    ///
    /// Not a fatal condition for a scanner: it just means "no ID3v2 tag here",
    /// which is how [`crate::find_id3v2`] reports absence.
    NotAnId3Tag,
    /// A major version this crate does not implement (only 2 is).
    UnsupportedMajorVersion {
        /// The major version byte found.
        major: u8,
    },
    /// A revision this crate does not implement.
    UnsupportedRevision {
        /// The revision byte found.
        revision: u8,
    },
    /// The declared tag or frame size exceeded the available bytes, or the
    /// size was absurd (the spec caps a tag at 256 MiB).
    SizeTooLarge {
        /// The declared size.
        declared: u32,
        /// The bytes actually available.
        available: usize,
    },
    /// A frame ran past the end of its declared length.
    TruncatedFrame {
        /// The frame id, as four bytes (or three, for v2.2).
        id: FrameId,
        /// Bytes the frame claimed.
        expected: usize,
        /// Bytes actually present.
        got: usize,
    },
    /// A frame's declared size was zero or otherwise unusable.
    InvalidFrameSize {
        /// The frame id.
        id: FrameId,
        /// The offending size.
        size: u32,
    },
    /// A text frame's encoding byte was not one of 0–3.
    InvalidTextEncoding {
        /// The encoding byte found.
        encoding: u8,
    },
    /// A frame's declared encoding does not match the bytes that follow, e.g.
    /// UTF-16 with no byte-order mark.
    InvalidTextData {
        /// The frame id being decoded.
        id: FrameId,
        /// What was wrong.
        reason: &'static str,
    },
    /// A synch-safe integer used a byte with the high bit set.
    ///
    /// Synch-safe encoding packs 7 bits per byte; a byte above 0x7F means the
    /// writer used a plain integer where the spec required synch-safe. That is
    /// common in the wild, so callers may choose to recover — see
    /// [`crate::synchsafe::decode_lenient`].
    NotSynchSafe {
        /// The offending byte value.
        byte: u8,
    },
    /// An ID3v1 tag was exactly 128 bytes but its magic did not match.
    NotAnId3v1Tag,
    /// A numeric field could not be parsed.
    InvalidNumber {
        /// The field name.
        field: &'static str,
        /// The text that failed to parse.
        value: alloc::string::String,
    },
    /// A `CHAP`/`CTOC` frame's element id was not terminated by a null byte.
    UnterminatedElementId,
    /// UTF-16 data contained an unpaired surrogate.
    InvalidUtf16,
    /// A string was not valid UTF-8 where the spec required it.
    InvalidUtf8,
}

impl fmt::Display for Id3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Id3Error::NotAnId3Tag => write!(f, "input does not start with the `ID3` identifier"),
            Id3Error::UnsupportedMajorVersion { major } => {
                write!(f, "ID3v{major} is not supported (only v2)")
            }
            Id3Error::UnsupportedRevision { revision } => {
                write!(f, "ID3v2 revision {revision} is not supported")
            }
            Id3Error::SizeTooLarge {
                declared,
                available,
            } => write!(
                f,
                "declared size {declared} exceeds the {available} available byte(s)"
            ),
            Id3Error::TruncatedFrame { id, expected, got } => write!(
                f,
                "frame {} claims {expected} byte(s) but only {got} remain",
                id.as_str()
            ),
            Id3Error::InvalidFrameSize { id, size } => {
                write!(f, "frame {} has an unusable size of {size}", id.as_str())
            }
            Id3Error::InvalidTextEncoding { encoding } => {
                write!(f, "text encoding byte {encoding} is not one of 0-3")
            }
            Id3Error::InvalidTextData { id, reason } => {
                write!(f, "frame {} has invalid text: {reason}", id.as_str())
            }
            Id3Error::NotSynchSafe { byte } => {
                write!(f, "byte 0x{byte:02X} is not synch-safe (high bit set)")
            }
            Id3Error::NotAnId3v1Tag => write!(f, "no `TAG` magic at the expected offset"),
            Id3Error::InvalidNumber { field, value } => {
                write!(f, "field `{field}` is not a number: {value:?}")
            }
            Id3Error::UnterminatedElementId => {
                write!(f, "element id is not null-terminated")
            }
            Id3Error::InvalidUtf16 => write!(f, "UTF-16 data contains an unpaired surrogate"),
            Id3Error::InvalidUtf8 => write!(f, "string is not valid UTF-8"),
        }
    }
}

impl core::error::Error for Id3Error {}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic
    )]
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn display_is_stable() {
        assert_eq!(
            Id3Error::NotAnId3Tag.to_string(),
            "input does not start with the `ID3` identifier"
        );
        assert_eq!(
            Id3Error::UnsupportedMajorVersion { major: 4 }.to_string(),
            "ID3v4 is not supported (only v2)"
        );
        assert_eq!(
            Id3Error::SizeTooLarge {
                declared: 9999,
                available: 10
            }
            .to_string(),
            "declared size 9999 exceeds the 10 available byte(s)"
        );
        assert_eq!(
            Id3Error::NotSynchSafe { byte: 0xFF }.to_string(),
            "byte 0xFF is not synch-safe (high bit set)"
        );
    }

    #[test]
    fn is_std_error() {
        fn assert_error<E: core::error::Error>(_: &E) {}
        assert_error(&Id3Error::NotAnId3v1Tag);
    }
}
