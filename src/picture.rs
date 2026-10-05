//! Attached pictures (`APIC`, and the v2.2 `PIC`).
//!
//! Cover art is the largest frame in a typical tag — often the whole point of
//! writing one — and the most commonly malformed. The spec's MIME type is a
//! null-terminated string terminated by *two* nulls when a description
//! follows, which is a reliable source of off-by-one bugs, so the parser here
//! is strict about it and reports which field it failed on.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;
use crate::frame::FrameId;
use crate::text::TextEncoding;

/// The picture's role, per the ID3v2 attached-picture type table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PictureType {
    /// Other.
    #[default]
    Other,
    /// 32×32 PNG file icon.
    Icon,
    /// Other file icon.
    OtherIcon,
    /// Cover (front).
    CoverFront,
    /// Cover (back).
    CoverBack,
    /// Leaflet page.
    Leaflet,
    /// Media (e.g. the label side of a CD).
    Media,
    /// Lead artist / lead performer / soloist.
    LeadArtist,
    /// Artist / performer.
    Artist,
    /// Conductor.
    Conductor,
    /// Band / orchestra.
    Band,
    /// Composer.
    Composer,
    /// Lyricist / text writer.
    Lyricist,
    /// Recording location.
    RecordingLocation,
    /// During recording.
    DuringRecording,
    /// During performance.
    DuringPerformance,
    /// Movie / video screen capture.
    ScreenCapture,
    /// A bright coloured fish. The spec really does define this.
    BrightFish,
    /// Illustration.
    Illustration,
    /// Band / artist logotype.
    Logotype,
    /// Publisher / studio logotype.
    PublisherLogotype,
    /// A type this crate does not name.
    Unrecognised(u8),
}

impl PictureType {
    /// The value as it appears on the wire.
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            PictureType::Other => 0x00,
            PictureType::Icon => 0x01,
            PictureType::OtherIcon => 0x02,
            PictureType::CoverFront => 0x03,
            PictureType::CoverBack => 0x04,
            PictureType::Leaflet => 0x05,
            PictureType::Media => 0x06,
            PictureType::LeadArtist => 0x07,
            PictureType::Artist => 0x08,
            PictureType::Conductor => 0x09,
            PictureType::Band => 0x0A,
            PictureType::Composer => 0x0B,
            PictureType::Lyricist => 0x0C,
            PictureType::RecordingLocation => 0x0D,
            PictureType::DuringRecording => 0x0E,
            PictureType::DuringPerformance => 0x0F,
            PictureType::ScreenCapture => 0x10,
            PictureType::BrightFish => 0x11,
            PictureType::Illustration => 0x12,
            PictureType::Logotype => 0x13,
            PictureType::PublisherLogotype => 0x14,
            PictureType::Unrecognised(b) => b,
        }
    }

    /// Decode a wire value.
    #[must_use]
    pub fn from_byte(b: u8) -> PictureType {
        match b {
            0x00 => PictureType::Other,
            0x01 => PictureType::Icon,
            0x02 => PictureType::OtherIcon,
            0x03 => PictureType::CoverFront,
            0x04 => PictureType::CoverBack,
            0x05 => PictureType::Leaflet,
            0x06 => PictureType::Media,
            0x07 => PictureType::LeadArtist,
            0x08 => PictureType::Artist,
            0x09 => PictureType::Conductor,
            0x0A => PictureType::Band,
            0x0B => PictureType::Composer,
            0x0C => PictureType::Lyricist,
            0x0D => PictureType::RecordingLocation,
            0x0E => PictureType::DuringRecording,
            0x0F => PictureType::DuringPerformance,
            0x10 => PictureType::ScreenCapture,
            0x11 => PictureType::BrightFish,
            0x12 => PictureType::Illustration,
            0x13 => PictureType::Logotype,
            0x14 => PictureType::PublisherLogotype,
            other => PictureType::Unrecognised(other),
        }
    }

    /// Whether this is one of the three cover slots, which is what a library
    /// wants to select on.
    #[must_use]
    pub fn is_cover(self) -> bool {
        matches!(
            self,
            PictureType::CoverFront | PictureType::CoverBack | PictureType::Media
        )
    }
}

/// A decoded attached picture.
///
/// `data` is the raw encoded image. Decoding JPEG or PNG is deliberately out of
/// scope for an L0 crate: this one has no dependencies and no image knowledge,
/// and the estate's `media-kit` already sniffs and validates image payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedPicture {
    /// The encoding of `description`.
    pub description_encoding: TextEncoding,
    /// The MIME type, e.g. `image/jpeg`. Lower-cased by the spec.
    pub mime_type: String,
    /// The picture's role.
    pub picture_type: PictureType,
    /// Free-text description, often the empty string.
    pub description: String,
    /// The encoded image bytes.
    pub data: Vec<u8>,
}

impl AttachedPicture {
    /// Parse an `APIC` (v2.3/2.4) or `PIC` (v2.2) body.
    ///
    /// # Errors
    ///
    /// [`Id3Error::InvalidTextData`] if the encoding byte or MIME type is
    /// malformed or unterminated.
    pub fn parse(id: FrameId, body: &[u8], major_version: u8) -> Result<Self, Id3Error> {
        // v2.2's PIC is a 3-byte image format instead of a MIME string, and
        // the description always follows. Represent it as the equivalent MIME
        // type so callers see one shape.
        let v22 = major_version == 2;

        let mut cursor = Cursor::new(body);
        let encoding_byte = cursor.take_byte().ok_or(Id3Error::InvalidTextData {
            id,
            reason: "picture frame has no encoding byte",
        })?;
        let encoding = TextEncoding::from_byte(encoding_byte)?;

        let mime_type = if v22 {
            // "PNG" / "JPG" with an implicit major format.
            let format = cursor.take_until_nul().ok_or(Id3Error::InvalidTextData {
                id,
                reason: "v2.2 picture format is not terminated",
            })?;
            match format {
                b"PNG" => String::from("image/png"),
                b"JPG" => String::from("image/jpeg"),
                other => {
                    let mut s = String::from("image/");
                    for b in other {
                        s.push(char::from(*b).to_ascii_lowercase());
                    }
                    s
                }
            }
        } else {
            let mime = cursor.take_until_nul().ok_or(Id3Error::InvalidTextData {
                id,
                reason: "picture MIME type is not null-terminated",
            })?;
            String::from_utf8(mime.to_vec()).map_err(|_| Id3Error::InvalidUtf8)?
        };

        let picture_type = PictureType::from_byte(cursor.take_byte().unwrap_or(0x00));

        // The description is null-terminated and the image bytes follow it, so
        // the split point is the FIRST null after the picture-type byte — not
        // a trailing one.
        //
        // For UTF-16 the terminator is a NUL *code unit*, so it is two zero
        // bytes. Scanning for a single zero byte instead would split inside the
        // first character, since every ASCII letter is followed by a zero byte
        // in UTF-16.
        let rest = cursor.remainder();
        let unit = if matches!(encoding, TextEncoding::Utf16Bom | TextEncoding::Utf16Be) {
            2
        } else {
            1
        };
        let terminator_at = (0..rest.len()).step_by(unit).find(|i| {
            rest.get(*i..*i + unit)
                .is_some_and(|w| w.iter().all(|b| *b == 0x00))
        });
        let (description_bytes, data) = match terminator_at {
            Some(nul) => (
                rest.get(..nul).unwrap_or(&[]),
                rest.get(nul + unit..).unwrap_or(&[]),
            ),
            // No terminator: the frame is truncated. Treat the whole remainder
            // as the image with no description rather than losing the art.
            None => (rest.get(..0).unwrap_or(&[]), rest),
        };
        let description = if description_bytes.is_empty() {
            String::new()
        } else {
            encoding.decode(description_bytes).unwrap_or_default()
        };

        Ok(Self {
            description_encoding: encoding,
            mime_type,
            picture_type,
            description,
            data: data.to_vec(),
        })
    }

    /// Serialize to an `APIC` body.
    ///
    /// # Errors
    ///
    /// Never in practice; the signature is fallible for symmetry with the other
    /// frame writers.
    pub fn to_body(&self) -> Result<Vec<u8>, Id3Error> {
        let mut out = Vec::with_capacity(self.data.len() + self.mime_type.len() + 8);
        out.push(self.description_encoding.as_byte());
        out.extend_from_slice(self.mime_type.as_bytes());
        out.push(0x00);
        out.push(self.picture_type.as_byte());
        out.extend_from_slice(&self.description_encoding.encode(&self.description));
        // The terminator must match the description's width: one NUL for
        // Latin-1/UTF-8, a NUL *code unit* for UTF-16. Writing a single byte
        // for UTF-16 leaves the reader scanning into the image data.
        let terminator = if matches!(
            self.description_encoding,
            TextEncoding::Utf16Bom | TextEncoding::Utf16Be
        ) {
            2
        } else {
            1
        };
        out.extend(core::iter::repeat_n(0x00u8, terminator));
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    /// Build a front-cover picture from an encoded image.
    ///
    /// The MIME type is derived from a magic-number sniff of the first few
    /// bytes, so a caller does not have to know whether the encoder wrote JPEG
    /// or PNG. Unrecognised data falls back to `application/octet-stream`, which
    /// is what the spec's "other" case calls for.
    #[must_use]
    pub fn cover(data: Vec<u8>) -> Self {
        Self {
            description_encoding: TextEncoding::Latin1,
            mime_type: String::from(sniff_mime(&data)),
            picture_type: PictureType::CoverFront,
            description: String::new(),
            data,
        }
    }
}

/// Sniff an image MIME type from its leading bytes.
#[must_use]
pub fn sniff_mime(data: &[u8]) -> &'static str {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif"
    } else if data.starts_with(b"BM") {
        "image/bmp"
    } else if data.len() >= 12 && data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// A bounds-checked forward cursor over a frame body.
///
/// Written out rather than using indices because the crate denies
/// `clippy::indexing_slicing`, and a picture frame has enough sequential
/// field reads that `get(i + n)` chains would be unreadable.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take_byte(&mut self) -> Option<u8> {
        let b = self.data.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    /// Consume up to and including the next null, returning the bytes before it.
    fn take_until_nul(&mut self) -> Option<&'a [u8]> {
        let start = self.pos;
        let mut i = self.pos;
        while let Some(b) = self.data.get(i) {
            if *b == 0x00 {
                self.pos = i + 1;
                return self.data.get(start..i);
            }
            i += 1;
        }
        None
    }

    fn remainder(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
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

    /// A minimal single-pixel PNG, enough to exercise the sniffer.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x00,
    ];
    /// A JPEG SOI + APP0 prefix.
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    #[test]
    fn mime_sniffing_covers_the_common_formats() {
        assert_eq!(sniff_mime(PNG), "image/png");
        assert_eq!(sniff_mime(JPEG), "image/jpeg");
        assert_eq!(sniff_mime(b"GIF89a...."), "image/gif");
        assert_eq!(sniff_mime(b"BM...."), "image/bmp");
        assert_eq!(sniff_mime(&[]), "application/octet-stream");
        assert_eq!(
            sniff_mime(b"not an image at all"),
            "application/octet-stream"
        );
    }

    #[test]
    fn webp_needs_twelve_bytes() {
        let mut riff = Vec::from(*b"RIFF");
        riff.extend_from_slice(&[0, 0, 0, 0]);
        riff.extend_from_slice(b"WEBP");
        assert_eq!(sniff_mime(&riff), "image/webp");
        // Too short to hold the WEBP tag: must not panic.
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WE"), "application/octet-stream");
    }

    #[test]
    fn picture_round_trips_with_a_description() {
        let pic = AttachedPicture {
            description_encoding: TextEncoding::Latin1,
            mime_type: String::from("image/png"),
            picture_type: PictureType::CoverFront,
            description: String::from("front"),
            data: PNG.to_vec(),
        };
        let body = pic.to_body().expect("serializes");
        let back = AttachedPicture::parse(FrameId::new(*b"APIC").expect("valid"), &body, 4)
            .expect("reparses");
        assert_eq!(back.mime_type, "image/png");
        assert_eq!(back.picture_type, PictureType::CoverFront);
        assert_eq!(back.description, "front");
        assert_eq!(back.data, PNG);
    }

    #[test]
    fn picture_round_trips_without_a_description() {
        // The spec's minimal form: ... picture type, then one trailing null.
        let mut body = vec![0u8];
        body.extend_from_slice(b"image/jpeg\0");
        body.push(PictureType::CoverFront.as_byte());
        body.push(0x00);
        body.extend_from_slice(JPEG);
        let pic = AttachedPicture::parse(FrameId::new(*b"APIC").expect("valid"), &body, 4)
            .expect("reparses");
        assert_eq!(pic.description, "");
        assert_eq!(pic.data, JPEG);
    }

    #[test]
    fn picture_rejects_an_unterminated_mime() {
        let body = vec![0u8, b'i', b'm', b'a', b'g', b'e', b'/']; // no null
        assert!(matches!(
            AttachedPicture::parse(FrameId::new(*b"APIC").expect("valid"), &body, 4),
            Err(Id3Error::InvalidTextData { .. })
        ));
    }

    #[test]
    fn picture_rejects_an_empty_body() {
        assert!(AttachedPicture::parse(FrameId::new(*b"APIC").expect("valid"), &[], 4).is_err());
    }

    #[test]
    fn v22_pic_maps_format_to_mime() {
        let mut body = vec![0u8];
        body.extend_from_slice(b"PNG\0");
        body.push(0x03);
        body.push(0x00);
        body.extend_from_slice(PNG);
        let pic = AttachedPicture::parse(FrameId::new_v22(*b"PIC").expect("valid"), &body, 2)
            .expect("reparses");
        assert_eq!(pic.mime_type, "image/png");
        assert_eq!(pic.data, PNG);
    }

    #[test]
    fn v22_jpg_maps_to_jpeg() {
        let mut body = vec![0u8];
        body.extend_from_slice(b"JPG\0");
        body.push(0x03);
        body.push(0x00);
        body.extend_from_slice(JPEG);
        let pic = AttachedPicture::parse(FrameId::new_v22(*b"PIC").expect("valid"), &body, 2)
            .expect("reparses");
        assert_eq!(pic.mime_type, "image/jpeg");
    }

    #[test]
    fn cover_helper_sniffs() {
        assert_eq!(AttachedPicture::cover(PNG.to_vec()).mime_type, "image/png");
        assert_eq!(
            AttachedPicture::cover(JPEG.to_vec()).mime_type,
            "image/jpeg"
        );
        assert_eq!(
            AttachedPicture::cover(PNG.to_vec()).picture_type,
            PictureType::CoverFront
        );
    }

    #[test]
    fn picture_types_round_trip_and_classify() {
        for b in 0x00..=0x14u8 {
            assert_eq!(PictureType::from_byte(b).as_byte(), b);
        }
        assert_eq!(
            PictureType::from_byte(0x99),
            PictureType::Unrecognised(0x99)
        );
        assert!(PictureType::CoverFront.is_cover());
        assert!(PictureType::CoverBack.is_cover());
        assert!(!PictureType::Icon.is_cover());
    }

    #[test]
    fn picture_round_trips_a_utf16_description() {
        let pic = AttachedPicture {
            description_encoding: TextEncoding::Utf16Bom,
            mime_type: String::from("image/png"),
            picture_type: PictureType::CoverBack,
            description: String::from("Grüße"),
            data: PNG.to_vec(),
        };
        let body = pic.to_body().expect("serializes");
        let back = AttachedPicture::parse(FrameId::new(*b"APIC").expect("valid"), &body, 4)
            .expect("reparses");
        assert_eq!(back.description, "Grüße");
        assert_eq!(back.description_encoding, TextEncoding::Utf16Bom);
    }

    #[test]
    fn cursor_take_until_nul_handles_consecutive_nuls() {
        let mut c = Cursor::new(b"ab\0\0cd\0");
        assert_eq!(c.take_until_nul(), Some(&b"ab"[..]));
        assert_eq!(c.take_until_nul(), Some(&b""[..]));
        assert_eq!(c.take_until_nul(), Some(&b"cd"[..]));
        assert_eq!(c.take_until_nul(), None);
        assert_eq!(c.remainder(), &b""[..]);
    }
}
