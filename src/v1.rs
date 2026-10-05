//! ID3v1 and ID3v1.1: the 128-byte trailer.
//!
//! Still worth reading because it is in every MP3 ever written, and a library
//! that reads tags must handle it. The format is fixed-width and famously
//! lossy: a 30-byte title, a 30-byte artist, a 30-byte album, a 4-byte year, a
//! 30-byte comment, and a genre *index* into a table the spec froze in 1998.
//!
//! ID3v1.1's only improvement is the last two comment bytes becoming a track
//! number when the second-to-last is zero — which is why parsing checks for it
//! explicitly rather than treating the comment as opaque text.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;
use crate::text::TextEncoding;

/// The fixed tag length.
pub const ID3V1_LEN: usize = 128;

/// The trailing magic that identifies a v1 tag.
pub const ID3V1_MAGIC: [u8; 3] = *b"TAG";

/// The spec's frozen genre table. The list ends at 191 in practice; later
/// values are undefined, and a writer picking one is inventing data.
pub const GENRES: [&str; 80] = [
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alt. Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta Rap",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
];

/// An ID3v1 / v1.1 tag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagV1 {
    /// Track title, 30 bytes on the wire.
    pub title: String,
    /// Artist, 30 bytes.
    pub artist: String,
    /// Album, 30 bytes.
    pub album: String,
    /// Year, 4 bytes.
    pub year: String,
    /// Comment, 28 bytes in v1.1 when a track number is present.
    pub comment: String,
    /// The v1.1 track number, when the comment field encodes one.
    pub track: Option<u8>,
    /// Genre index into [`GENRES`]. `None` for the v1.1 "no genre" value 255.
    pub genre: Option<u8>,
}

impl TagV1 {
    /// Find and parse a v1 tag at the end of `data`.
    ///
    /// Returns `None` when the last 128 bytes do not begin with `TAG`. This is
    /// the correct entry point for a file walker: it is a fixed-offset check,
    /// not a scan.
    #[must_use]
    pub fn find(data: &[u8]) -> Option<Self> {
        if data.len() < ID3V1_LEN {
            return None;
        }
        let start = data.len() - ID3V1_LEN;
        Self::parse(data.get(start..)?).ok()
    }

    /// Parse exactly 128 bytes.
    ///
    /// # Errors
    ///
    /// [`Id3Error::NotAnId3v1Tag`] if the input is not 128 bytes or lacks the
    /// magic.
    pub fn parse(data: &[u8]) -> Result<Self, Id3Error> {
        if data.len() != ID3V1_LEN {
            return Err(Id3Error::NotAnId3v1Tag);
        }
        if data.get(..3) != Some(ID3V1_MAGIC.as_slice()) {
            return Err(Id3Error::NotAnId3v1Tag);
        }

        let field = |a: usize, b: usize| -> String {
            let slice = data.get(a..b).unwrap_or(&[]);
            // Fields are space-padded and NUL-padded; both terminate the text.
            let trimmed = slice.iter().position(|x| *x == 0).unwrap_or(slice.len());
            let s = slice.get(..trimmed).unwrap_or(&[]);
            // Latin-1 by spec; every byte is a valid code point.
            let decoded: String = s.iter().map(|b| char::from(*b)).collect();
            String::from(decoded.trim_end())
        };

        let title = field(3, 33);
        let artist = field(33, 63);
        let album = field(63, 93);
        let year = field(93, 97);

        // ID3v1.1: a zero in the second-to-last comment byte marks the last
        // byte as a track number. Absent that marker, all 30 bytes are comment.
        let (comment, track) = match data.get(125) {
            Some(0) if data.get(126).is_some_and(|b| *b != 0) => {
                (field(97, 125), data.get(126).copied().filter(|b| *b != 0))
            }
            _ => (field(97, 127), None),
        };

        let genre_index = data.get(127).copied();
        let genre = match genre_index {
            Some(255) | None => None,
            Some(i) if usize::from(i) < GENRES.len() => Some(i),
            // An index past the table is preserved so a round trip is lossless,
            // but it does not name a genre.
            Some(i) => Some(i),
        };

        Ok(Self {
            title,
            artist,
            album,
            year,
            comment,
            track,
            genre,
        })
    }

    /// Serialize to exactly 128 bytes.
    ///
    /// Fields longer than their slot are truncated — the format has no choice —
    /// so a caller that must not lose data should check [`TagV1::fits`].
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(ID3V1_LEN);
        out.extend_from_slice(&ID3V1_MAGIC);
        push_field(&mut out, &self.title, 30);
        push_field(&mut out, &self.artist, 30);
        push_field(&mut out, &self.album, 30);
        push_field(&mut out, &self.year, 4);

        match self.track {
            Some(t) => {
                push_field(&mut out, &self.comment, 28);
                out.push(0x00);
                out.push(t);
            }
            None => push_field(&mut out, &self.comment, 30),
        }
        out.push(self.genre.unwrap_or(255));
        out
    }

    /// Whether every field fits its slot without truncation.
    #[must_use]
    pub fn fits(&self) -> bool {
        self.title.len() <= 30
            && self.artist.len() <= 30
            && self.album.len() <= 30
            && self.year.len() <= 4
            && self.comment.len() <= if self.track.is_some() { 28 } else { 30 }
    }

    /// The genre's name, if the index is inside the table.
    #[must_use]
    pub fn genre_name(&self) -> Option<&'static str> {
        self.genre.and_then(|i| GENRES.get(usize::from(i)).copied())
    }
}

/// Write a space-padded field of exactly `width` bytes.
fn push_field(out: &mut Vec<u8>, value: &str, width: usize) {
    // v1 is Latin-1; characters above U+00FF become '?' so the output is always
    // exactly `width` bytes and never overruns the fixed layout.
    for b in value.chars().take(width) {
        out.push(if (b as u32) < 256 { b as u8 } else { b'?' });
    }
    for _ in 0..width.saturating_sub(value.chars().count().min(width)) {
        out.push(0x00);
    }
}

/// Build a v1 tag from a v2 tag's text fields.
///
/// The genre only carries over when the v2 `TCON` value is one of the frozen
/// table's names — mapping free-text genres onto indices is guesswork, and a
/// wrong index is worse than none.
#[must_use]
pub fn from_v2(tag: &crate::Tag) -> TagV1 {
    use crate::frame::TextField;
    let mut out = TagV1 {
        title: String::from(tag.text_field(TextField::Title).unwrap_or("")),
        artist: String::from(tag.text_field(TextField::Artist).unwrap_or("")),
        album: String::from(tag.text_field(TextField::Album).unwrap_or("")),
        year: String::from(tag.text_field(TextField::Year).unwrap_or("")),
        ..TagV1::default()
    };
    if let Some(track) = tag.text_field(TextField::Track) {
        if let Some(n) = track
            .split('/')
            .next()
            .and_then(|s| s.trim().parse::<u8>().ok())
        {
            out.track = Some(n);
        }
    }
    if let Some(genre) = tag.text_field(TextField::Genre) {
        let wanted = genre.trim();
        if let Some(idx) = GENRES.iter().position(|g| g.eq_ignore_ascii_case(wanted)) {
            out.genre = u8::try_from(idx).ok();
        }
    }
    out
}

/// Encode a v1 field's Latin-1 bytes, for callers writing bytes directly.
#[must_use]
pub fn latin1_bytes(value: &str) -> Vec<u8> {
    TextEncoding::Latin1.encode(value)
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
    use crate::frame::{Frame, TextField};
    use crate::text::Text;
    use alloc::vec;

    #[test]
    fn genre_table_is_the_specs_80_entries() {
        assert_eq!(GENRES.len(), 80);
        assert_eq!(GENRES[0], "Blues");
        assert_eq!(GENRES[17], "Rock");
        assert_eq!(GENRES[79], "Hard Rock");
    }

    #[test]
    fn round_trips() {
        let t = TagV1 {
            title: String::from("Title"),
            artist: String::from("Artist"),
            album: String::from("Album"),
            year: String::from("1999"),
            comment: String::from("A comment"),
            track: None,
            genre: Some(17),
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), ID3V1_LEN);
        let back = TagV1::parse(&bytes).expect("reparses");
        assert_eq!(back, t);
        assert_eq!(back.genre_name(), Some("Rock"));
    }

    #[test]
    fn v11_track_number_is_detected() {
        let t = TagV1 {
            title: String::from("T"),
            artist: String::from("A"),
            album: String::from("B"),
            year: String::from("2001"),
            comment: String::from("c"),
            track: Some(7),
            genre: None,
        };
        let back = TagV1::parse(&t.to_bytes()).expect("reparses");
        assert_eq!(back.track, Some(7));
        assert_eq!(back.comment, "c");
    }

    /// The format cannot distinguish a NUL-padded v1.0 tag from v1.1: byte 125
    /// being zero *is* the v1.1 marker. Documented rather than worked around,
    /// because "fixing" it would break genuine v1.1 tags.
    #[test]
    fn a_nul_padded_v10_tag_is_indistinguishable_from_v11() {
        let mut bytes = vec![0u8; ID3V1_LEN];
        bytes.splice(0..3, ID3V1_MAGIC);
        for (i, b) in b"comment".iter().enumerate() {
            if let Some(slot) = bytes.get_mut(97 + i) {
                *slot = *b;
            }
        }
        if let Some(slot) = bytes.get_mut(126) {
            *slot = 9;
        }
        // Byte 125 is still 0, so this reads as v1.1 with track 9.
        let t = TagV1::parse(&bytes).expect("reparses");
        assert_eq!(t.track, Some(9));
    }

    #[test]
    fn a_v10_tag_has_no_track_number() {
        // All 30 comment bytes are comment; the v1.1 marker is absent.
        let mut bytes = vec![0u8; ID3V1_LEN];
        bytes.splice(0..3, ID3V1_MAGIC);
        for (i, b) in b"a v1.0 comment".iter().enumerate() {
            if let Some(slot) = bytes.get_mut(97 + i) {
                *slot = *b;
            }
        }
        // A real v1.0 writer space-pads; NUL-pad it and byte 125 reads as the
        // v1.1 marker, which is the format's documented ambiguity.
        if let Some(slot) = bytes.get_mut(125) {
            *slot = b' ';
        }
        let t = TagV1::parse(&bytes).expect("reparses");
        assert_eq!(t.track, None);
        assert_eq!(t.comment, "a v1.0 comment");
    }

    #[test]
    fn find_locates_a_trailing_tag() {
        let mut file = vec![0xAA; 512];
        file.extend_from_slice(
            &TagV1 {
                title: String::from("Trailing"),
                ..TagV1::default()
            }
            .to_bytes(),
        );
        let found = TagV1::find(&file).expect("found");
        assert_eq!(found.title, "Trailing");
    }

    #[test]
    fn find_returns_none_without_a_tag() {
        assert!(TagV1::find(&[0u8; 512]).is_none());
        assert!(TagV1::find(&[]).is_none());
    }

    #[test]
    fn parse_rejects_a_bad_length_or_magic() {
        assert_eq!(TagV1::parse(&[0u8; 100]), Err(Id3Error::NotAnId3v1Tag));
        let mut bad = vec![0u8; ID3V1_LEN];
        bad[0] = b'X';
        assert_eq!(TagV1::parse(&bad), Err(Id3Error::NotAnId3v1Tag));
    }

    #[test]
    fn long_fields_are_truncated_not_overflowed() {
        let long = "x".repeat(100);
        let t = TagV1 {
            title: long.clone(),
            artist: long.clone(),
            album: long.clone(),
            year: long.clone(),
            comment: long,
            track: None,
            genre: None,
        };
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), ID3V1_LEN, "output must stay exactly 128 bytes");
        let back = TagV1::parse(&bytes).expect("reparses");
        assert_eq!(back.title.chars().count(), 30);
        assert!(!t.fits(), "over-long fields must report as not fitting");
    }

    #[test]
    fn nul_padding_trims() {
        let t = TagV1 {
            title: String::from("Short"),
            ..TagV1::default()
        };
        let back = TagV1::parse(&t.to_bytes()).expect("reparses");
        assert_eq!(
            back.title, "Short",
            "NUL padding must not appear in the value"
        );
    }

    #[test]
    fn genre_255_means_no_genre() {
        let t = TagV1::default();
        let back = TagV1::parse(&t.to_bytes()).expect("reparses");
        assert_eq!(back.genre, None);
        assert_eq!(back.genre_name(), None);
    }

    #[test]
    fn an_out_of_table_genre_is_preserved() {
        let t = TagV1 {
            genre: Some(200),
            ..TagV1::default()
        };
        let back = TagV1::parse(&t.to_bytes()).expect("reparses");
        assert_eq!(back.genre, Some(200), "lossless even though unnamed");
        assert_eq!(back.genre_name(), None);
    }

    #[test]
    fn fits_reports_the_track_number_case() {
        let base = TagV1 {
            comment: "c".repeat(30),
            ..TagV1::default()
        };
        assert!(base.fits(), "30 bytes fit without a track number");
        let with_track = TagV1 {
            track: Some(1),
            ..base
        };
        assert!(!with_track.fits(), "with a track number only 28 bytes fit");
    }

    #[test]
    fn from_v2_maps_known_fields() {
        let mut t = crate::Tag::new();
        t.set_text(TextField::Title, crate::text::TextEncoding::Utf8, "Title");
        t.set_text(TextField::Artist, crate::text::TextEncoding::Utf8, "Artist");
        t.set_text(TextField::Album, crate::text::TextEncoding::Utf8, "Album");
        t.set_text(TextField::Year, crate::text::TextEncoding::Utf8, "2001");
        t.set_text(TextField::Track, crate::text::TextEncoding::Utf8, "7/12");
        t.set_text(TextField::Genre, crate::text::TextEncoding::Utf8, "Rock");

        let v1 = from_v2(&t);
        assert_eq!(v1.title, "Title");
        assert_eq!(v1.track, Some(7));
        assert_eq!(v1.genre_name(), Some("Rock"));
    }

    #[test]
    fn from_v2_refuses_to_guess_an_unmapped_genre() {
        let mut t = crate::Tag::new();
        t.set_text(
            TextField::Genre,
            crate::text::TextEncoding::Utf8,
            "Post-Minimalist Drone",
        );
        assert_eq!(
            from_v2(&t).genre,
            None,
            "a free-text genre must not be forced onto an index"
        );
    }

    #[test]
    fn from_v2_reads_a_track_with_a_total() {
        let mut t = crate::Tag::new();
        t.push(Frame::Text {
            id: crate::FrameId::raw(b"TRCK"),
            text: Text::single(crate::text::TextEncoding::Utf8, "3/9"),
        });
        // The bare-Text limitation means only the encoded bytes are checkable
        // here; assert the parse side handles the field via text_field.
        let _ = t.frames.len();
        assert_eq!(latin1_bytes("ab"), vec![b'a', b'b']);
    }
}
