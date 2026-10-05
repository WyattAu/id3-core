//! Frame identifiers and the frame body union.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;
use crate::picture::AttachedPicture;
use crate::text::{Text, TextEncoding};

/// A four-character (v2.3/2.4) or three-character (v2.2) frame identifier.
///
/// Stored as four bytes with v2.2 ids left-padded by a null, so `TIT` becomes
/// `b"\0TIT"`. That keeps one representation for all three versions and makes
/// the v2.3→v2.4 id upgrade table a pure lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId([u8; 4]);

impl FrameId {
    /// Build an id from four ASCII bytes.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidTextData`] if any byte is not an uppercase letter or
    /// digit — the spec restricts frame ids to `A-Z0-9`.
    pub fn new(bytes: [u8; 4]) -> Result<Self, Id3Error> {
        for b in bytes {
            if !(b.is_ascii_uppercase() || b.is_ascii_digit()) {
                return Err(Id3Error::InvalidTextData {
                    id: FrameId::unknown(),
                    reason: "frame id must be A-Z or 0-9",
                });
            }
        }
        Ok(Self(bytes))
    }

    /// Build an id without validating, for error paths that must name something.
    #[must_use]
    pub const fn unknown() -> Self {
        Self([0, 0, 0, 0])
    }

    /// Build an id from four bytes with no validation.
    ///
    /// For ids read off the wire, where the bytes have already been accepted by
    /// the parser and a rejected id would lose a frame the caller may still
    /// want to inspect. Prefer [`FrameId::new`] for ids you are constructing.
    #[must_use]
    pub const fn raw(bytes: &[u8; 4]) -> Self {
        Self(*bytes)
    }

    /// Build a v2.2 three-character id, padding on the left.
    ///
    /// The pad byte is added *after* validation — validating it would reject
    /// every v2.2 id, since NUL is not an `A-Z0-9` character.
    ///
    /// # Errors
    ///
    /// As [`FrameId::new`].
    pub fn new_v22(bytes: [u8; 3]) -> Result<Self, Id3Error> {
        for b in bytes {
            if !(b.is_ascii_uppercase() || b.is_ascii_digit()) {
                return Err(Id3Error::InvalidTextData {
                    id: FrameId::unknown(),
                    reason: "frame id must be A-Z or 0-9",
                });
            }
        }
        Ok(Self([0, bytes[0], bytes[1], bytes[2]]))
    }

    /// The raw four bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    /// Whether this id came from a v2.2 tag (null-padded).
    #[must_use]
    pub fn is_v22(&self) -> bool {
        self.0[0] == 0
    }

    /// The id as a string, with the v2.2 padding removed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Frame ids are ASCII by construction; the only non-ASCII-capable path
        // is an `unknown` id, which is all NULs and still valid UTF-8.
        core::str::from_utf8(&self.0)
            .unwrap_or("????")
            .trim_start_matches('\0')
    }

    /// The v2.3/v2.4 name for a v2.2 id, if the spec defines one.
    ///
    /// The v2.3 release renamed every three-character id to four. Frames with
    /// no defined successor keep their padded form and are surfaced as
    /// `Unknown`.
    #[must_use]
    pub fn upgrade_from_v22(self) -> FrameId {
        if !self.is_v22() {
            return self;
        }
        match self.as_str() {
            "BUF" => four(b"RFDB"),
            "CNT" => four(b"PCNT"),
            "COM" => four(b"COMM"),
            "CRA" => four(b"AENC"),
            "CRM" => four(b"ENCR"),
            "ETC" => four(b"ETCO"),
            "EQU" => four(b"EQUA"),
            "GEO" => four(b"GEOB"),
            "IPL" => four(b"IPLS"),
            "LNK" => four(b"LINK"),
            "MCI" => four(b"MCID"),
            "MLL" => four(b"MLLT"),
            "PIC" => four(b"APIC"),
            "POP" => four(b"POPM"),
            "REV" => four(b"RVAD"),
            "RVA" => four(b"RVAD"),
            "SLT" => four(b"SYLT"),
            "STC" => four(b"SYTC"),
            "TAL" => four(b"TALB"),
            "TBP" => four(b"TBPM"),
            "TCM" => four(b"TCOM"),
            "TCO" => four(b"TCON"),
            "TCR" => four(b"TCOP"),
            "TDA" => four(b"TDAT"),
            "TDY" => four(b"TDLY"),
            "TEN" => four(b"TENC"),
            "TFT" => four(b"TFLT"),
            "TKE" => four(b"TKEY"),
            "TLA" => four(b"TLAN"),
            "TLE" => four(b"TLEN"),
            "TMT" => four(b"TMED"),
            "TOA" => four(b"TOPE"),
            "TOF" => four(b"TOFN"),
            "TOL" => four(b"TOLY"),
            "TOR" => four(b"TORY"),
            "TOT" => four(b"TOAL"),
            "TP1" => four(b"TPE1"),
            "TP2" => four(b"TPE2"),
            "TP3" => four(b"TPE3"),
            "TP4" => four(b"TPE4"),
            "TPA" => four(b"TPOS"),
            "TPB" => four(b"TPUB"),
            "TRC" => four(b"TSRC"),
            "TRK" => four(b"TRCK"),
            "TSI" => four(b"TSIZ"),
            "TSS" => four(b"TSSE"),
            "TT1" => four(b"TIT1"),
            "TT2" => four(b"TIT2"),
            "TT3" => four(b"TIT3"),
            "TYE" => four(b"TYER"),
            "UFI" => four(b"UFID"),
            "ULT" => four(b"USLT"),
            "WAF" => four(b"WOAF"),
            "WAR" => four(b"WOAR"),
            "WAS" => four(b"WOAS"),
            _ => self,
        }
    }
}

/// Build a `FrameId` from a four-byte literal in a `const` context.
const fn four(bytes: &[u8; 4]) -> FrameId {
    FrameId([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl core::fmt::Display for FrameId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A parsed frame body.
///
/// Unknown frame ids become [`Frame::Unknown`] carrying the raw bytes rather
/// than an error: a tag written by a newer encoder must still yield the frames
/// this crate does understand.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Frame {
    /// A text frame (`T***`), decoded, with the id it arrived under.
    ///
    /// The id is part of the variant rather than looked up separately because a
    /// text frame *is* its id — `TIT2` and `TPE1` have identical bodies and
    /// nothing else in the value distinguishes them.
    Text {
        /// The frame id, e.g. `TIT2`.
        id: FrameId,
        /// The decoded text.
        text: Text,
    },
    /// An attached picture (`APIC`/`PIC`).
    Picture(AttachedPicture),
    /// A chapter (`CHAP`).
    Chapter(crate::chapter::Chapter),
    /// A table of contents (`CTOC`).
    Toc(crate::chapter::Toc),
    /// A user comment (`COMM`).
    Comment(crate::comment::Comment),
    /// An unsynchronised lyrics/text block (`USLT`).
    UnsynchronisedLyrics(crate::comment::UnsynchronisedLyrics),
    /// A frame this crate does not decode, with its id and raw body.
    Unknown {
        /// The frame id.
        id: FrameId,
        /// The undecoded body.
        data: Vec<u8>,
    },
}

impl Frame {
    /// The frame's id, whatever its variant.
    #[must_use]
    pub fn id(&self) -> FrameId {
        match self {
            Frame::Text { id, .. } => *id,
            Frame::Picture(_) => FrameId::raw(b"APIC"),
            Frame::Chapter(_) => FrameId::raw(b"CHAP"),
            Frame::Toc(_) => FrameId::raw(b"CTOC"),
            Frame::Comment(_) => FrameId::raw(b"COMM"),
            Frame::UnsynchronisedLyrics(_) => FrameId::raw(b"USLT"),
            Frame::Unknown { id, .. } => *id,
        }
    }
}

/// A frame's wire header plus body, as read from a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    /// The frame id.
    pub id: FrameId,
    /// The body bytes, after unsynchronisation has been undone.
    pub data: Vec<u8>,
}

/// Decode a frame body according to its id.
///
/// `major_version` selects the v2.2 three-character id table and the v2.4-only
/// encodings. An id this crate has no decoder for yields
/// [`Frame::Unknown`].
///
/// # Errors
///
/// Propagates the specific decoder's error. Callers scanning a file should
/// treat any error as "skip this frame" rather than "the tag is unusable".
pub fn decode(id: FrameId, body: &[u8], major_version: u8) -> Result<Frame, Id3Error> {
    let fourcc = if major_version == 2 {
        id.upgrade_from_v22()
    } else {
        id
    };

    match &fourcc.as_bytes()[..] {
        b"APIC" => Ok(Frame::Picture(AttachedPicture::parse(
            id,
            body,
            major_version,
        )?)),
        b"CHAP" => Ok(Frame::Chapter(crate::chapter::Chapter::parse(body)?)),
        b"CTOC" => Ok(Frame::Toc(crate::chapter::Toc::parse(body)?)),
        b"COMM" => Ok(Frame::Comment(crate::comment::Comment::parse(
            body,
            major_version,
        )?)),
        b"USLT" => Ok(Frame::UnsynchronisedLyrics(
            crate::comment::UnsynchronisedLyrics::parse(body, major_version)?,
        )),
        // `TXXX` starts with `T` but is NOT a text frame: its body is a
        // description then a value, with no encoding selector. Routing it
        // through the text decoder reads the first byte of the description as
        // an encoding and rejects almost every real TXXX.
        b"TXXX" | b"UFID" | b"PRIV" | b"WOAF" | b"WOAR" | b"WOAS" => Ok(Frame::Unknown {
            id: fourcc,
            data: body.to_vec(),
        }),
        _ if fourcc.as_bytes()[0] == b'T' => Ok(Frame::Text {
            id: fourcc,
            text: Text::parse(fourcc, body)?,
        }),
        _ => Ok(Frame::Unknown {
            id: fourcc,
            data: body.to_vec(),
        }),
    }
}

/// The standard text frames, and whether each is v2.4's multi-value form.
///
/// Useful for a tagger that wants to present "title", "artist", "album" etc.
/// without hard-coding ids, and for validating that a value went into the
/// right frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextField {
    /// `TIT2` — title/subtitle/swapped title.
    Title,
    /// `TPE1` — lead artist.
    Artist,
    /// `TALB` — album/movie/show title.
    Album,
    /// `TRCK` — track number / total tracks.
    Track,
    /// `TPOS` — disc number / total discs.
    Disc,
    /// `TYER` (v2.3) or `TDRC` (v2.4) — year.
    Year,
    /// `TCON` — genre.
    Genre,
    /// `TPE2` — album artist.
    AlbumArtist,
    /// `TCOM` — composer.
    Composer,
    /// `TPUB` — publisher / label.
    Publisher,
    /// `TSRC` — ISRC.
    Isrc,
    /// `TBPM` — beats per minute.
    Bpm,
}

impl TextField {
    /// The frame id for this field at the given tag version.
    ///
    /// `TYER` became `TDRC` in v2.4; both are accepted on read, and the writer
    /// emits the right one for the version it is writing.
    #[must_use]
    pub fn frame_id(self, major_version: u8) -> FrameId {
        let bytes: [u8; 4] = match self {
            TextField::Title => *b"TIT2",
            TextField::Artist => *b"TPE1",
            TextField::Album => *b"TALB",
            TextField::Track => *b"TRCK",
            TextField::Disc => *b"TPOS",
            TextField::Year => {
                if major_version >= 4 {
                    *b"TDRC"
                } else {
                    *b"TYER"
                }
            }
            TextField::Genre => *b"TCON",
            TextField::AlbumArtist => *b"TPE2",
            TextField::Composer => *b"TCOM",
            TextField::Publisher => *b"TPUB",
            TextField::Isrc => *b"TSRC",
            TextField::Bpm => *b"TBPM",
        };
        FrameId::new(bytes).unwrap_or_else(|_| FrameId::unknown())
    }

    /// Every id this field may be stored under, most specific first.
    ///
    /// Lookup and clearing both walk this list, so a value written as `TYER` is
    /// found (and replaced) when asked for as `Year`, and a v2.2 tag's
    /// three-character id resolves to its four-character name.
    #[must_use]
    pub fn frame_ids(self) -> Vec<FrameId> {
        let mut ids = Vec::with_capacity(4);
        // The v2.3 spelling, then the v2.4 one; `Year` changed between them.
        let v23 = self.frame_id(3);
        let v24 = self.frame_id(4);
        if v24 != v23 {
            ids.push(v24);
            ids.push(v23);
        } else {
            ids.push(v24);
        }
        if let Some(v22) = self.v22_id() {
            ids.push(v22);
        }
        ids
    }

    /// This field's v2.2 three-character id, where the spec defines one.
    #[must_use]
    pub fn v22_id(self) -> Option<FrameId> {
        let three: &[u8; 3] = match self {
            TextField::Title => b"TT2",
            TextField::Artist => b"TP1",
            TextField::Album => b"TAL",
            TextField::Track => b"TRK",
            TextField::Disc => b"TPA",
            TextField::Year => b"TYE",
            TextField::Genre => b"TCO",
            TextField::AlbumArtist => b"TP2",
            TextField::Composer => b"TCM",
            TextField::Publisher => b"TPB",
            TextField::Isrc => b"TRC",
            TextField::Bpm => b"TBP",
        };
        FrameId::new_v22(*three).ok()
    }

    /// Every field, in the order a tagger would present them.
    #[must_use]
    pub fn all() -> Vec<TextField> {
        alloc::vec![
            TextField::Title,
            TextField::Artist,
            TextField::Album,
            TextField::AlbumArtist,
            TextField::Track,
            TextField::Disc,
            TextField::Year,
            TextField::Genre,
            TextField::Composer,
            TextField::Publisher,
            TextField::Isrc,
            TextField::Bpm,
        ]
    }
}

/// Encode a text frame body for `field`.
#[must_use]
pub fn encode_text_field(
    field: TextField,
    major_version: u8,
    encoding: TextEncoding,
    value: &str,
) -> (FrameId, Vec<u8>) {
    let id = field.frame_id(major_version);
    let body = Text::single(encoding, value).to_body();
    match body {
        Ok(b) => (id, b),
        // `to_body` cannot currently fail; keep the signature infallible for
        // callers building a tag in a loop.
        Err(_) => (id, alloc::vec![encoding.as_byte()]),
    }
}

/// A convenience alias for a decoded text frame's first value.
pub type TextValue = String;

/// A picture type re-export, so a caller working with pictures does not need a
/// second import.
pub use crate::picture::PictureType as ArtworkType;
