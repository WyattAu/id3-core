//! Comments (`COMM`) and unsynchronised lyrics (`USLT`).
//!
//! Both frames are the same shape with different defaults: a three-byte
//! language code, a short description, then the text. `COMM`'s description is
//! conventionally `iTunNORM`-style tooling metadata or empty; `USLT`'s is a
//! content descriptor ("Lyrics", "transcription").

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;
use crate::frame::FrameId;
use crate::text::{Text, TextEncoding};

/// An ISO-639-2 three-letter language code, as three ASCII bytes.
///
/// Kept as bytes rather than a `String` because the spec requires exactly three
/// and a validator should not have to re-check it. Three NUL bytes is the
/// spec's "unknown" value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Language([u8; 3]);

impl Language {
    /// Build from three ASCII bytes (lowercase letters per the spec).
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidTextData`] if a byte is not an ASCII letter.
    pub fn new(bytes: [u8; 3]) -> Result<Self, Id3Error> {
        for b in bytes {
            if !b.is_ascii_alphabetic() {
                return Err(Id3Error::InvalidTextData {
                    id: FrameId::unknown(),
                    reason: "language code must be three ASCII letters",
                });
            }
        }
        Ok(Self([
            bytes[0].to_ascii_lowercase(),
            bytes[1].to_ascii_lowercase(),
            bytes[2].to_ascii_lowercase(),
        ]))
    }

    /// The spec's "unknown language" code.
    #[must_use]
    pub const fn undetermined() -> Self {
        Self([0, 0, 0])
    }

    /// The raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 3] {
        &self.0
    }

    /// The code as a string, or `None` for the undetermined value.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if self.0 == [0, 0, 0] {
            return None;
        }
        core::str::from_utf8(&self.0).ok()
    }

    /// Whether this is the undetermined code.
    #[must_use]
    pub fn is_undetermined(&self) -> bool {
        self.0 == [0, 0, 0]
    }
}

/// A decoded comment (`COMM`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// The language the text is in.
    pub language: Language,
    /// Short content descriptor, conventionally empty.
    pub description: String,
    /// The comment text.
    pub text: Text,
}

impl Comment {
    /// A comment in `language` with no descriptor.
    #[must_use]
    pub fn new(language: Language, text: &str, encoding: TextEncoding) -> Self {
        Self {
            language,
            description: String::new(),
            text: Text::single(encoding, text),
        }
    }

    /// Parse a `COMM` body.
    ///
    /// # Errors
    ///
    /// [`Id3Error::TruncatedFrame`] if the 4-byte fixed header is short, and
    /// the text decoder's error for malformed content.
    pub fn parse(body: &[u8], _major_version: u8) -> Result<Self, Id3Error> {
        // COMM layout: encoding byte, then the 3-byte language, then the
        // descriptor. The encoding comes FIRST — writing them the other way
        // round produces a file whose first language byte is read as an
        // encoding selector, which is how this crate's own tests caught it.
        let header = body.get(..4).ok_or(Id3Error::TruncatedFrame {
            id: frame_id(b"COMM"),
            expected: 4,
            got: body.len(),
        })?;
        let encoding_byte = header.first().copied().unwrap_or(0);
        let encoding = TextEncoding::from_byte(encoding_byte)?;
        let language = match header.get(1..4).and_then(|s| <[u8; 3]>::try_from(s).ok()) {
            Some([a, b, c]) => Language([a, b, c]),
            None => Language::undetermined(),
        };

        let rest = body.get(4..).unwrap_or(&[]);
        let nul = rest.iter().position(|b| *b == 0x00);
        let (desc_bytes, text_bytes) = match nul {
            Some(n) => (
                rest.get(..n).unwrap_or(&[]),
                rest.get(n + 1..).unwrap_or(&[]),
            ),
            // No terminator: the whole remainder is the text.
            None => (rest.get(..0).unwrap_or(&[]), rest),
        };
        let description = if desc_bytes.is_empty() {
            String::new()
        } else {
            encoding.decode(desc_bytes).unwrap_or_default()
        };
        // The text has NO encoding byte of its own — the frame's byte applies
        // to the descriptor and the text alike. Going through `Text::parse`
        // would consume the first character of the comment as a second
        // encoding selector.
        let text = Text {
            encoding,
            values: alloc::vec![encoding.decode(text_bytes).unwrap_or_default()],
        };

        Ok(Self {
            language,
            description,
            text,
        })
    }

    /// Serialize to a `COMM` body.
    ///
    /// # Errors
    ///
    /// Propagates the text encoder's error, which cannot currently fail.
    pub fn to_body(&self) -> Result<Vec<u8>, Id3Error> {
        let mut out = Vec::with_capacity(16);
        out.push(self.text.encoding.as_byte());
        out.extend_from_slice(self.language.as_bytes());
        out.extend_from_slice(&self.text.encoding.encode(&self.description));
        out.push(0x00);
        out.extend_from_slice(&self.text.encoding.encode(self.text.text()));
        Ok(out)
    }
}

/// A decoded unsynchronised-lyrics frame (`USLT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsynchronisedLyrics {
    /// The language the lyrics are in.
    pub language: Language,
    /// Content descriptor, e.g. "Lyrics".
    pub description: String,
    /// The lyrics, or a transcription.
    pub text: Text,
}

impl UnsynchronisedLyrics {
    /// Lyrics in `language` with the conventional descriptor.
    #[must_use]
    pub fn new(language: Language, lyrics: &str, encoding: TextEncoding) -> Self {
        Self {
            language,
            description: String::from("Lyrics"),
            text: Text::single(encoding, lyrics),
        }
    }

    /// Parse a `USLT` body.
    ///
    /// # Errors
    ///
    /// As [`Comment::parse`].
    pub fn parse(body: &[u8], _major_version: u8) -> Result<Self, Id3Error> {
        // USLT shares COMM's layout: encoding byte, then language.
        let header = body.get(..4).ok_or(Id3Error::TruncatedFrame {
            id: frame_id(b"USLT"),
            expected: 4,
            got: body.len(),
        })?;
        let encoding = TextEncoding::from_byte(header.first().copied().unwrap_or(0))?;
        let language = match header.get(1..4).and_then(|s| <[u8; 3]>::try_from(s).ok()) {
            Some([a, b, c]) => Language([a, b, c]),
            None => Language::undetermined(),
        };

        let rest = body.get(4..).unwrap_or(&[]);
        let nul = rest.iter().position(|b| *b == 0x00);
        let (desc_bytes, text_bytes) = match nul {
            Some(n) => (
                rest.get(..n).unwrap_or(&[]),
                rest.get(n + 1..).unwrap_or(&[]),
            ),
            None => (rest.get(..0).unwrap_or(&[]), rest),
        };
        let description = if desc_bytes.is_empty() {
            String::new()
        } else {
            encoding.decode(desc_bytes).unwrap_or_default()
        };

        // As with COMM, the frame-level encoding byte covers the whole body.
        Ok(Self {
            language,
            description,
            text: Text {
                encoding,
                values: alloc::vec![encoding.decode(text_bytes).unwrap_or_default()],
            },
        })
    }

    /// Serialize to a `USLT` body.
    ///
    /// # Errors
    ///
    /// Propagates the text encoder's error, which cannot currently fail.
    pub fn to_body(&self) -> Result<Vec<u8>, Id3Error> {
        let mut out = Vec::with_capacity(16);
        out.push(self.text.encoding.as_byte());
        out.extend_from_slice(self.language.as_bytes());
        out.extend_from_slice(&self.text.encoding.encode(&self.description));
        out.push(0x00);
        out.extend_from_slice(&self.text.encoding.encode(self.text.text()));
        Ok(out)
    }
}

const fn frame_id(s: &[u8; 4]) -> FrameId {
    FrameId::raw(s)
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
    fn language_validates_and_lowercases() {
        assert_eq!(Language::new(*b"ENG"), Ok(Language(*b"eng")));
        assert!(Language::new(*b"e1g").is_err());
        assert!(Language::new([0, 0, 0]).is_err(), "NUL is not a letter");
    }

    #[test]
    fn undetermined_language() {
        let l = Language::undetermined();
        assert!(l.is_undetermined());
        assert_eq!(l.as_str(), None);
        assert_eq!(Language::new(*b"deu").expect("valid").as_str(), Some("deu"));
    }

    #[test]
    fn comment_round_trips() {
        let c = Comment::new(
            Language::new(*b"eng").expect("valid"),
            "A great book",
            TextEncoding::Utf8,
        );
        let back = Comment::parse(&c.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.text.text(), "A great book");
        assert_eq!(back.language.as_str(), Some("eng"));
        assert_eq!(back.description, "");
    }

    #[test]
    fn comment_keeps_a_descriptor() {
        let mut c = Comment::new(
            Language::new(*b"eng").expect("valid"),
            "note",
            TextEncoding::Utf8,
        );
        c.description = String::from("iTunNORM");
        let back = Comment::parse(&c.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.description, "iTunNORM");
        assert_eq!(back.text.text(), "note");
    }

    #[test]
    fn comment_with_empty_text_is_legal() {
        // A blank comment field is common; it must not be an error.
        let c = Comment::new(Language::undetermined(), "", TextEncoding::Latin1);
        let back = Comment::parse(&c.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.text.text(), "");
        assert!(back.language.is_undetermined());
    }

    #[test]
    fn comment_rejects_a_short_header() {
        assert!(matches!(
            Comment::parse(&[0, b'e', b'n'], 4),
            Err(Id3Error::TruncatedFrame { .. })
        ));
    }

    #[test]
    fn comment_tolerates_a_missing_descriptor_terminator() {
        // Body with no NUL after the header: the whole rest is the text.
        // Layout is encoding byte, then the 3-byte language.
        let mut body = vec![0x00, b'e', b'n', b'g'];
        body.extend_from_slice(b"dangling");
        let c = Comment::parse(&body, 4).expect("tolerant");
        assert_eq!(c.description, "");
        assert_eq!(c.text.text(), "dangling");
    }

    #[test]
    fn lyrics_round_trip() {
        let l = UnsynchronisedLyrics::new(
            Language::new(*b"eng").expect("valid"),
            "la la la",
            TextEncoding::Utf8,
        );
        let back =
            UnsynchronisedLyrics::parse(&l.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.text.text(), "la la la");
        assert_eq!(back.description, "Lyrics");
    }

    #[test]
    fn lyrics_round_trip_multiline() {
        let text = "line one\nline two\n\nline four";
        let l = UnsynchronisedLyrics::new(
            Language::new(*b"eng").expect("valid"),
            text,
            TextEncoding::Utf8,
        );
        let back =
            UnsynchronisedLyrics::parse(&l.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.text.text(), text);
    }

    #[test]
    fn lyrics_reject_a_short_header() {
        assert!(UnsynchronisedLyrics::parse(&[], 4).is_err());
    }

    #[test]
    fn utf16_comment_round_trips() {
        let c = Comment::new(
            Language::new(*b"deu").expect("valid"),
            "Grüße aus Köln",
            TextEncoding::Utf16Bom,
        );
        let back = Comment::parse(&c.to_body().expect("serializes"), 4).expect("reparses");
        assert_eq!(back.text.text(), "Grüße aus Köln");
    }
}
