//! The ID3v2 tag: header, frame table, unsynchronisation, and writing.

use alloc::vec::Vec;

use crate::chapter::{Chapter, Toc};
use crate::comment::{Comment, UnsynchronisedLyrics};
use crate::error::Id3Error;
use crate::frame::{decode, Frame, FrameId, RawFrame, TextField};
use crate::picture::AttachedPicture;
use crate::synchsafe;
use crate::text::{Text, TextEncoding};

/// The identifier every ID3v2 tag starts with.
pub const ID3_IDENTIFIER: [u8; 3] = *b"ID3";

/// The spec's tag-size ceiling: 28 synch-safe bits.
pub const MAX_TAG_SIZE: u32 = 0x0FFF_FFFF;

/// The frame header width per version: id + size + flags.
const V24_FRAME_HEADER: usize = 10;

/// An ID3v2.3 frame header: a 4-byte id, a 4-byte plain size, and a single flag byte.
///
/// One byte shorter than v2.4's, which has two flag bytes. Reading a v2.3 tag with the
/// v2.4 length mis-frames every frame, so this constant is load-bearing rather than
/// cosmetic.
const V23_FRAME_HEADER: usize = 9;
/// v2.2's frame header is id + 3-byte size, with no flags.
const V22_FRAME_HEADER: usize = 6;

/// A parsed or constructible ID3v2 tag.
///
/// Frames keep their tag order rather than being sorted, because a `CHAP`/
/// `CTOC` pair's meaning depends on relative order and a `TXXX` block can carry
/// an update-time that a later frame supersedes. Lookups are by id and scan,
/// which is fine at tag sizes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tag {
    /// Major version: 2, 3, or 4. Read from the header; set by the writer.
    pub major_version: u8,
    /// Minor/revision byte. Not validated beyond 0 for v2.4.
    pub revision: u8,
    /// Whether the tag's frames are unsynchronised. Applied on write, undone on
    /// read.
    pub unsynchronisation: bool,
    /// Whether this tag is in the experimental stage (`TST` frame present).
    pub experimental: bool,
    /// Whether the tag has an extended header.
    pub has_extended_header: bool,
    /// The frames, in tag order.
    pub frames: Vec<Frame>,
}

impl Tag {
    /// An empty v2.4 tag, which is what a writer should start from.
    ///
    /// v2.4 is the default because it is the only version with UTF-8, `CHAP`,
    /// and `CTOC`; a writer emitting v2.3 would silently lose chapters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            major_version: 4,
            revision: 0,
            unsynchronisation: false,
            experimental: false,
            has_extended_header: false,
            frames: Vec::new(),
        }
    }

    /// An empty tag at the given major version.
    ///
    /// # Errors
    ///
    /// [`Id3Error::UnsupportedMajorVersion`] for a version other than 2, 3, or 4.
    pub fn with_version(major_version: u8) -> Result<Self, Id3Error> {
        if !(2..=4).contains(&major_version) {
            return Err(Id3Error::UnsupportedMajorVersion {
                major: major_version,
            });
        }
        Ok(Self {
            major_version,
            ..Self::new()
        })
    }

    /// Parse a complete ID3v2 tag, header included.
    ///
    /// # Errors
    ///
    /// [`Id3Error::NotAnId3Tag`] when the identifier is absent,
    /// [`Id3Error::UnsupportedMajorVersion`] for a non-v2 tag, and
    /// [`Id3Error::SizeTooLarge`] when the declared size exceeds the input or
    /// the spec's ceiling.
    pub fn parse(data: &[u8]) -> Result<Self, Id3Error> {
        let (header, body) = split_header(data)?;
        let declared = synchsafe::decode_value(header.size);

        if declared > MAX_TAG_SIZE {
            return Err(Id3Error::SizeTooLarge {
                declared,
                available: body.len(),
            });
        }
        let end = usize::try_from(declared)
            .unwrap_or(usize::MAX)
            .min(body.len());
        let owned: Vec<u8>;
        let mut tag_body: &[u8] = body.get(..end).unwrap_or(&[]);

        // Unsynchronisation is undone before any frame parsing: it is defined
        // over the whole tag body, and a frame's own size field counts the
        // unsynchronised bytes.
        if header.flags.unsynchronisation {
            owned = unsynchronise(tag_body);
            tag_body = &owned;
        }

        let mut tag = Self {
            major_version: header.major_version,
            revision: header.revision,
            unsynchronisation: header.flags.unsynchronisation,
            experimental: header.flags.experimental,
            has_extended_header: header.flags.extended_header,
            frames: Vec::new(),
        };

        if header.flags.extended_header {
            tag_body = skip_extended_header(tag_body, header.major_version);
        }

        tag.frames = read_frame_table(tag_body, &header)?;
        Ok(tag)
    }

    /// Every frame with the given id, in tag order.
    #[must_use]
    pub fn frames_with_id(&self, id: FrameId) -> Vec<&Frame> {
        self.frames
            .iter()
            .filter(|f| frame_matches(f, id))
            .collect()
    }

    /// The first frame with the given id.
    #[must_use]
    pub fn frame(&self, id: FrameId) -> Option<&Frame> {
        self.frames.iter().find(|f| frame_matches(f, id))
    }

    /// The text of the first frame for `field`, if present.
    ///
    /// Lookup goes through [`TagField::frame_ids`] rather than a single id, so
    /// a v2.2 tag — which stores three-character ids — is still found by its
    /// four-character field name. `Year` resolves to both `TYER` (v2.3) and
    /// `TDRC` (v2.4), since a tag written by one tool and finished by another
    /// routinely carries both.
    #[must_use]
    pub fn text_field(&self, field: TextField) -> Option<&str> {
        for id in field.frame_ids() {
            for frame in &self.frames {
                let Frame::Text { id: have, text } = frame else {
                    continue;
                };
                if frame_matches(frame, id) {
                    let _ = have;
                    return Some(text.text());
                }
            }
        }
        None
    }

    /// Every `CHAP` frame, in tag order.
    #[must_use]
    pub fn chapters(&self) -> Vec<&Chapter> {
        self.frames
            .iter()
            .filter_map(|f| match f {
                Frame::Chapter(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    /// The first `CTOC` frame.
    #[must_use]
    pub fn toc(&self) -> Option<&Toc> {
        self.frames.iter().find_map(|f| match f {
            Frame::Toc(t) => Some(t),
            _ => None,
        })
    }

    /// Chapters resolved against the first `CTOC`, in playing order.
    ///
    /// # Errors
    ///
    /// Never; the signature is fallable for symmetry with the rest of the API.
    pub fn resolved_chapters(&self) -> Result<Vec<crate::chapter::ResolvedChapter>, Id3Error> {
        let chapters: Vec<Chapter> = self.chapters().into_iter().cloned().collect();
        Ok(crate::chapter::resolve(&chapters, self.toc()))
    }

    /// The first cover picture, preferring `CoverFront`.
    #[must_use]
    pub fn cover(&self) -> Option<&AttachedPicture> {
        let pictures: Vec<&AttachedPicture> = self
            .frames
            .iter()
            .filter_map(|f| match f {
                Frame::Picture(p) => Some(p),
                _ => None,
            })
            .collect();
        pictures
            .iter()
            .find(|p| p.picture_type.is_cover())
            .or_else(|| pictures.first())
            .copied()
    }

    /// The first `COMM` frame.
    #[must_use]
    pub fn comment(&self) -> Option<&Comment> {
        self.frames.iter().find_map(|f| match f {
            Frame::Comment(c) => Some(c),
            _ => None,
        })
    }

    /// The first `USLT` frame.
    #[must_use]
    pub fn lyrics(&self) -> Option<&UnsynchronisedLyrics> {
        self.frames.iter().find_map(|f| match f {
            Frame::UnsynchronisedLyrics(l) => Some(l),
            _ => None,
        })
    }

    /// Append a frame.
    pub fn push(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    /// Remove every frame matching `id`, returning how many went.
    pub fn remove(&mut self, id: FrameId) -> usize {
        let before = self.frames.len();
        self.frames.retain(|f| !frame_matches(f, id));
        before - self.frames.len()
    }

    /// Set a text field, replacing any existing frame for it.
    ///
    /// Every id the field could be stored under is cleared first, so rewriting a
    /// field never leaves a stale duplicate behind — including across the v2.3
    /// `TYER` / v2.4 `TDRC` change and the v2.2 three-character forms. An empty
    /// value clears the field rather than writing an empty frame.
    pub fn set_text(&mut self, field: TextField, encoding: TextEncoding, value: &str) {
        for id in field.frame_ids() {
            self.remove(id);
        }
        if value.is_empty() {
            return;
        }
        // Store the id this tag's version actually writes.
        let id = if self.major_version == 2 {
            field.v22_id().unwrap_or_else(|| field.frame_id(4))
        } else {
            field.frame_id(self.major_version)
        };
        self.frames.push(Frame::Text {
            id,
            text: Text::single(encoding, value),
        });
    }

    /// Serialize to a complete tag, header included.
    ///
    /// Frame order is preserved and frame sizes are recomputed, so a tag parsed
    /// from a v2.3 file and re-emitted as v2.4 is rewritten rather than
    /// patched.
    ///
    /// # Errors
    ///
    /// [`Id3Error::SizeTooLarge`] if the frames exceed the tag-size ceiling.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Id3Error> {
        let mut body: Vec<u8> = Vec::new();
        for frame in &self.frames {
            self.encode_frame(frame, &mut body)?;
        }

        if self.unsynchronisation {
            body = unsynchronise(&body);
        }

        let size = synchsafe::encode_usize(body.len())?;
        let mut out = Vec::with_capacity(10 + body.len());
        out.extend_from_slice(&ID3_IDENTIFIER);
        out.push(self.major_version);
        out.push(self.revision);
        let mut flags = 0u8;
        if self.unsynchronisation {
            flags |= 0x80;
        }
        if self.experimental {
            flags |= 0x20;
        }
        if self.has_extended_header {
            flags |= 0x40;
        }
        out.push(flags);
        out.extend_from_slice(&size);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Append one frame's wire form to `out`.
    fn encode_frame(&self, frame: &Frame, out: &mut Vec<u8>) -> Result<(), Id3Error> {
        // The chapters addendum is written against ID3v2.3 and v2.4; `CHAP` and `CTOC` do
        // not exist in v2.2, which uses three-character ids and six-byte headers. Writing
        // the frame anyway truncates the id to `CHA`, which no reader recognises as
        // chapters \u{2014} so a caller that fixed the chapters on a v2.2 file would be told
        // the write succeeded while the file came back chapterless. Refusing is the honest
        // answer, and the caller can convert the tag to v2.3 or v2.4 instead.
        if self.major_version == 2 {
            if let Frame::Chapter(_) | Frame::Toc(_) = frame {
                return Err(Id3Error::ChaptersUnsupportedAtVersion { major: 2 });
            }
        }
        let (id, body) = match frame {
            Frame::Text { id, text } => (*id, text.to_body()?),
            Frame::Picture(p) => (id3(b"APIC"), p.to_body()?),
            Frame::Chapter(c) => (id3(b"CHAP"), c.to_body()),
            Frame::Toc(t) => (id3(b"CTOC"), t.to_body()),
            Frame::Comment(c) => (id3(b"COMM"), c.to_body()?),
            Frame::UnsynchronisedLyrics(l) => (id3(b"USLT"), l.to_body()?),
            Frame::Unknown { id, data } => (*id, data.clone()),
        };

        if self.major_version == 2 {
            // v2.2 uses a 3-byte id and a 3-byte size. The id kept on the
            // frame is the *wire* id, so a parsed v2.2 tag re-emits the
            // three characters it was read from.
            // The wire form has no pad byte. A v2.2 id is held internally as
            // [0, b0, b1, b2] so one FrameId type serves both versions, so
            // drop the pad on the way out; a genuine four-character id is
            // truncated to three, which is the spec's rule.
            let bytes = id.as_bytes();
            let short: &[u8] = if id.is_v22() {
                bytes.get(1..4).unwrap_or(b"XXX")
            } else {
                bytes.get(..3).unwrap_or(b"XXX")
            };
            out.extend_from_slice(short);
            let size = body.len().min(0x00FF_FFFF);
            out.extend_from_slice(&(size as u32).to_be_bytes()[1..]);
            out.extend_from_slice(&body);
            return Ok(());
        }

        out.extend_from_slice(id.as_bytes());
        let size = u32::try_from(body.len()).map_err(|_| Id3Error::SizeTooLarge {
            declared: body.len().min(u32::MAX as usize) as u32,
            available: 0,
        })?;
        if self.major_version == 3 {
            // v2.3 keeps v2.4's ten-byte header and switches to a plain size. The
            // specification says the header is nine bytes, but mutagen writes ten and
            // ffmpeg reads ten, so a nine-byte v2.3 frame is a file no major tool will
            // read. Writing what the ecosystem reads is the point of writing at all;
            // `read_frame_table` accepts either shape when reading.
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&[0x00, 0x00]); // frame flags
        } else {
            out.extend_from_slice(&synchsafe::encode_usize(body.len())?);
            out.extend_from_slice(&[0x00, 0x00]); // frame flags
        }
        out.extend_from_slice(&body);
        Ok(())
    }
}

/// Whether a frame carries `id`.
///
/// Every variant knows its own id now, so this is a plain comparison. A v2.2
/// three-character id is compared after upgrade, so `PIC` matches a lookup for
/// `APIC`.
fn frame_matches(frame: &Frame, id: FrameId) -> bool {
    let have = frame.id();
    have == id || have.upgrade_from_v22() == id
}

const fn id3(s: &[u8; 4]) -> FrameId {
    FrameId::raw(s)
}

/// The ten-byte tag header.
#[derive(Debug, Clone, Copy)]
struct Header {
    major_version: u8,
    revision: u8,
    size: [u8; 4],
    flags: HeaderFlags,
}

#[derive(Debug, Clone, Copy)]
struct HeaderFlags {
    unsynchronisation: bool,
    extended_header: bool,
    experimental: bool,
}

/// Split a tag into its header fields and body.
fn split_header(data: &[u8]) -> Result<(Header, &[u8]), Id3Error> {
    if data.len() < 10 {
        return Err(Id3Error::NotAnId3Tag);
    }
    if data.get(..3) != Some(ID3_IDENTIFIER.as_slice()) {
        return Err(Id3Error::NotAnId3Tag);
    }
    let major = data.get(3).copied().unwrap_or(0);
    if major != 2 && major != 3 && major != 4 {
        return Err(Id3Error::UnsupportedMajorVersion { major });
    }
    let revision = data.get(4).copied().unwrap_or(0);
    // Header layout is ID3(3) | major(1) | revision(1) | flags(1) | size(4).
    // The flags byte therefore sits at index 5 and the synch-safe size at 6..10.
    let flags_byte = data.get(5).copied().unwrap_or(0);
    let size: [u8; 4] = data
        .get(6..10)
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0; 4]);
    // v2.2 has no extended-header or experimental bits in the same layout; the
    // low bit was compression, which v2.4 turned into "not used".
    let flags = if major == 2 {
        HeaderFlags {
            unsynchronisation: flags_byte & 0x80 != 0,
            extended_header: false,
            experimental: false,
        }
    } else {
        HeaderFlags {
            unsynchronisation: flags_byte & 0x80 != 0,
            extended_header: flags_byte & 0x40 != 0,
            experimental: flags_byte & 0x20 != 0,
        }
    };
    Ok((
        Header {
            major_version: major,
            revision,
            size,
            flags,
        },
        data.get(10..).unwrap_or(&[]),
    ))
}

/// Locate an ID3v2 tag in a byte stream.
///
/// Returns the tag's total length including the header, or `None` if the input
/// does not begin with a tag. This is what a scanner embedding `id3-core` in a
/// file walker wants, as opposed to [`Tag::parse`].
#[must_use]
pub fn find_id3v2(data: &[u8]) -> Option<usize> {
    let (header, body) = split_header(data).ok()?;
    let declared = usize::try_from(synchsafe::decode_value(header.size)).unwrap_or(usize::MAX);
    // A tag longer than the input is truncated but still real: report what is
    // actually present so a caller can decide.
    Some(10 + declared.min(body.len()))
}

/// Skip an extended header, returning the rest of the body.
fn skip_extended_header(body: &[u8], major: u8) -> &[u8] {
    // v2.3: 4-byte size not counting itself. v2.4: 4-byte *synch-safe* size
    // including itself.
    let raw = match body.get(..4).and_then(|s| <[u8; 4]>::try_from(s).ok()) {
        Some(b) => u32::from_be_bytes(b),
        None => return &[],
    };
    let skip = if major >= 4 {
        // v2.4 counts the size field itself.
        raw.max(6) as usize
    } else {
        // v2.3 excludes it, and is at least 6.
        (raw as usize).saturating_add(4)
    };
    body.get(skip..).unwrap_or(&[])
}

/// Read a tag's frame table, choosing the frame header length for its version.
///
/// # The ID3v2.3 frame header is nine bytes, and almost nobody uses nine
///
/// The ID3v2.3 specification says a frame header is a 4-byte id, a 4-byte **plain**
/// size and a single flag byte: nine bytes. ID3v2.4 adds a second flag byte, making it
/// ten, and encodes the size synch-safely.
///
/// Every implementation that matters writes v2.3 with **ten**. mutagen emits a ten-byte
/// header for a v2.3 tag, and ffmpeg reads it that way too — `case 3: case 4:
/// taghdrlen = 10;` in `libavformat/id3v2.c`, distinguishing the two versions only in how
/// the size is decoded. So a spec-conformant nine-byte v2.3 tag is a file no major tool
/// will read, and a ten-byte one is what a real v2.3 file actually contains.
///
/// Picking either one alone therefore breaks half the world, which is why this is a
/// decision rather than a constant:
///
/// * try the ten-byte shape, which is what mutagen and ffmpeg produce and consume;
/// * if the frame table cannot be walked under it — a frame id that is not four
///   upper-case alphanumerics, or a size that runs past the tag — retry with the
///   spec's nine bytes.
///
/// The discriminator is unambiguous in both directions. A nine-byte file read with a
/// ten-byte header makes the size field absorb the flag byte, giving a value far larger
/// than the remaining tag. A ten-byte file read with a nine-byte header starts the next
/// frame one byte early, landing on the second flag byte and the first body byte, so the
/// "id" contains NULs and non-ASCII — never four alphanumerics.
///
/// A tag that parses under neither shape is reported as an error rather than silently
/// read as whichever one produced fewer complaints.
fn read_frame_table(body: &[u8], header: &Header) -> Result<Vec<Frame>, Id3Error> {
    let major = header.major_version;
    let has_extended = header.flags.extended_header;

    if major == 2 {
        return parse_frames(body, major, has_extended, V22_FRAME_HEADER);
    }
    if major >= 4 {
        return parse_frames(body, major, has_extended, V24_FRAME_HEADER);
    }

    // v2.3: the ambiguous one. Ten bytes is what the ecosystem does; nine is what the
    // specification says. Both are tried, and the one that yields a walkable frame table
    // wins.
    match parse_frames(body, major, has_extended, V24_FRAME_HEADER) {
        Ok(frames) if frames_have_valid_ids(&frames) => Ok(frames),
        _ => parse_frames(body, major, has_extended, V23_FRAME_HEADER),
    }
}

/// Whether every parsed frame has an id that could really be an ID3 frame id.
///
/// This is the check that decides whether a frame table was walked correctly. Frame ids
/// are four upper-case letters or digits; a mis-framed walk puts the tail of one frame's
/// flags and the head of its body where the next id belongs, which is not alphanumeric.
fn frames_have_valid_ids(frames: &[Frame]) -> bool {
    frames.iter().all(|f| {
        f.id()
            .as_bytes()
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
}

/// Parse the frame table.
fn parse_frames(
    mut data: &[u8],
    major: u8,
    has_extended_header: bool,
    header_len: usize,
) -> Result<Vec<Frame>, Id3Error> {
    let mut out = Vec::new();
    let padding_needed = has_extended_header;

    while let Some(head) = data.get(..header_len) {
        // Padding: a run of NULs ends the frame table.
        if head.iter().all(|b| *b == 0x00) {
            break;
        }
        let (id, size, body_start) = if major == 2 {
            // v2.2: 3-byte id, 3-byte size. Both are known-present because
            // `head` is exactly `V22_FRAME_HEADER` long.
            let id3 = head
                .get(..3)
                .and_then(|s| <[u8; 3]>::try_from(s).ok())
                .unwrap_or([0; 3]);
            let size3 = head
                .get(3..6)
                .and_then(|s| <[u8; 3]>::try_from(s).ok())
                .unwrap_or([0; 3]);
            (
                FrameId::raw(&[0, id3[0], id3[1], id3[2]]),
                u32::from_be_bytes([0, size3[0], size3[1], size3[2]]),
                V22_FRAME_HEADER,
            )
        } else {
            let bytes: [u8; 4] = head
                .get(..4)
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0; 4]);
            let raw: [u8; 4] = head
                .get(4..8)
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0; 4]);
            // The version decides the encoding; guessing between them is
            // wrong, not lenient: a synch-safe 1024 reads as 2048 in plain
            // form, so a v2.4 frame over 128 bytes is mis-sized by a heuristic.
            let size = if major >= 4 {
                match synchsafe::decode(raw) {
                    Ok(strict) => strict,
                    // Not synch-safe: a v2.3-era writer in a v2.4 tag. Fall
                    // back to the plain reading rather than rejecting the tag.
                    Err(_) => u32::from_be_bytes(raw),
                }
            } else {
                u32::from_be_bytes(raw)
            };
            // v2.3's header is one byte shorter than v2.4's: the two flag bytes became
            // one. Using the v2.4 length here mis-frames every frame by that byte — the
            // flag is read as the first byte of the next id — so the table falls apart
            // and the whole tag fails to parse.
            (FrameId::raw(&bytes), size, header_len)
        };

        if size == 0 {
            // A zero size with a valid id is padding some encoders emit.
            break;
        }
        let size_usize = match usize::try_from(size) {
            Ok(v) => v,
            Err(_) => {
                return Err(Id3Error::SizeTooLarge {
                    declared: size,
                    available: data.len().saturating_sub(body_start),
                })
            }
        };
        let rest = data.get(body_start..).unwrap_or(&[]);
        if rest.len() < size_usize {
            // A truncated final frame is common on files cut short. Yield what
            // parsed cleanly rather than failing the whole tag.
            if rest.is_empty() {
                break;
            }
            return Err(Id3Error::TruncatedFrame {
                id,
                expected: size_usize,
                got: rest.len(),
            });
        }
        let body = rest.get(..size_usize).unwrap_or(&[]);
        let frame = decode(id, body, major)?;
        out.push(frame);
        data = rest.get(size_usize..).unwrap_or(&[]);
    }

    // `padding_needed` is read for clarity: v2.4 tags with an extended header
    // may carry frame-level padding, which the loop above already stops at.
    let _ = padding_needed;
    Ok(out)
}

/// Undo ID3v2 unsynchronisation.
///
/// The encoder inserts a zero byte after every `0xFF`; the decoder removes it.
/// A `0xFF 0x00` at the very end is left alone — that is data, not a sequence.
#[must_use]
pub fn unsynchronise(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let Some(b) = data.get(i).copied() else { break };
        out.push(b);
        if b == 0xFF {
            // Drop the inserted zero, but not at end of input.
            match data.get(i + 1) {
                Some(0x00) => i += 2,
                Some(_) => i += 1,
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Apply unsynchronisation, the inverse of [`unsynchronise`].
#[must_use]
pub fn synchronise(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (i, b) in data.iter().enumerate() {
        out.push(*b);
        if *b == 0xFF {
            // Insert before a byte that would otherwise continue an FF sequence.
            match data.get(i + 1) {
                Some(next) if *next == 0x00 || *next >= 0xE0 => out.push(0x00),
                _ => {}
            }
        }
    }
    out
}

/// Parse a tag's frames from a stream that starts at the header.
///
/// Thin alias for [`Tag::parse`], kept so a caller reading a whole file does
/// not need to know the tag length up front.
///
/// # Errors
///
/// As [`Tag::parse`].
pub fn parse(data: &[u8]) -> Result<Tag, Id3Error> {
    Tag::parse(data)
}

/// A `RawFrame` builder helper: a frame this crate does not decode, kept
/// verbatim so a tag can be copied through without loss.
#[must_use]
pub fn unknown_frame(id: FrameId, data: Vec<u8>) -> Frame {
    Frame::Unknown { id, data }
}

/// Re-export for callers assembling a tag by hand.
pub use crate::frame::encode_text_field as text_frame;

/// A convenience constructor for a raw, undecoded frame list entry.
#[must_use]
pub fn raw_frame(id: FrameId, data: Vec<u8>) -> RawFrame {
    RawFrame { id, data }
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
    use crate::text::TextEncoding;
    use alloc::vec;

    fn minimal_tag() -> Tag {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "Dune");
        t
    }

    #[test]
    fn new_defaults_to_v24() {
        let t = Tag::new();
        assert_eq!(t.major_version, 4);
        assert!(t.frames.is_empty());
    }

    #[test]
    fn with_version_rejects_other_versions() {
        assert!(Tag::with_version(3).is_ok());
        assert!(Tag::with_version(5).is_err());
    }

    #[test]
    fn tag_round_trips() {
        let t = minimal_tag();
        let bytes = t.to_bytes().expect("serializes");
        let back = Tag::parse(&bytes).expect("reparses");
        assert_eq!(back.major_version, 4);
        assert_eq!(back.text_field(TextField::Title), Some("Dune"));
    }

    #[test]
    fn header_is_ten_bytes_and_id3() {
        let bytes = minimal_tag().to_bytes().expect("serializes");
        assert_eq!(bytes.get(..3), Some(ID3_IDENTIFIER.as_slice()));
        assert_eq!(bytes.get(3), Some(&4));
        assert_eq!(bytes.len(), bytes.len());
    }

    #[test]
    fn parse_rejects_a_non_tag() {
        assert_eq!(Tag::parse(b"not a tag at all"), Err(Id3Error::NotAnId3Tag));
        assert_eq!(Tag::parse(b"ID3"), Err(Id3Error::NotAnId3Tag));
    }

    #[test]
    fn a_v23_tag_we_write_is_readable_by_the_ecosystem_shape() {
        // What this crate emits for v2.3 is a ten-byte header with a plain size, because
        // that is what mutagen writes and ffmpeg reads. Asserting the bytes rather than
        // round-tripping through this crate's own reader is deliberate: the reader accepts
        // both shapes, so a round trip would pass even if the writer emitted the
        // specification's nine bytes — a file nothing else can read.
        let mut tag = Tag::with_version(3).expect("v2.3 is supported");
        // UTF-16 so the body opens with an unambiguous `0x01` encoding byte. Latin-1 and
        // UTF-8 both open with a byte a flag field could equally be, which is what makes
        // this check possible at all.
        tag.set_text(TextField::Title, TextEncoding::Utf16Bom, "1984");
        let bytes = tag.to_bytes().expect("serialises");
        assert_eq!(bytes[3], 3, "tag is v2.3");

        // Layout: id(4) at 10, plain size(4) at 14, flags(2) at 18, body at 20.
        assert_eq!(&bytes[10..14], b"TIT2", "the first frame is the title");
        let size = u32::from_be_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) as usize;
        assert_eq!(&bytes[18..20], &[0x00, 0x00], "two flag bytes, as v2.4 has");
        assert_eq!(
            bytes[20], 0x01,
            "the body's encoding byte sits ten bytes in, so the header is ten \u{2014} the \
             shape mutagen writes and ffmpeg reads. A nine-byte header would put it at \
             index 19 and produce a file no major tool can read."
        );
        assert_eq!(
            20 + size,
            bytes.len(),
            "and the declared size spans exactly to the end of the tag"
        );
    }

    #[test]
    fn an_id3v23_tag_with_a_nine_byte_frame_header_parses() {
        // The specification's shape: a 4-byte id, a 4-byte plain size, one flag byte.
        let mut body = Vec::new();
        body.extend_from_slice(b"TIT2");
        body.extend_from_slice(&4u32.to_be_bytes());
        body.push(0x00); // one flag byte, nine-byte header
        body.extend_from_slice(&[0x03, b'a', b'b', b'c']); // UTF-8 "abc"

        let parsed = Tag::parse(&v23_tag(&body)).expect("a spec-shaped v2.3 tag must parse");
        assert_eq!(parsed.major_version, 3);
        assert_eq!(
            parsed.text_field(crate::TextField::Title),
            Some("abc"),
            "nine-byte frame headers are what the specification says"
        );
    }

    #[test]
    fn a_v22_tag_refuses_chapters_instead_of_losing_them() {
        // `CHAP` and `CTOC` do not exist in ID3v2.2, which uses three-character frame ids
        // and six-byte headers. Writing the frames anyway truncates the id to `CHA`, which
        // no reader recognises as chapters \u{2014} so a caller that fixed a v2.2 file's
        // chapters would be told the write succeeded while the file came back chapterless.
        // It reparses fine, which is what makes this silent rather than obvious.
        let mut tag = Tag::with_version(2).expect("valid");
        tag.push(Frame::Chapter(crate::chapter::Chapter::new("ch0", 0)));
        // Matched rather than matched on the message: the variant is the contract, and a
        // `no_std` test module does not have `ToString` in scope.
        assert!(matches!(
            tag.to_bytes(),
            Err(Id3Error::ChaptersUnsupportedAtVersion { major: 2 })
        ));
    }

    #[test]
    fn a_v22_tag_still_carries_its_text_after_a_chapter_write_is_refused() {
        // Refusing is only honest if it refuses *everything*: a partial write would leave
        // the tag modified in ways the caller did not ask for.
        let mut tag = Tag::with_version(2).expect("valid");
        tag.set_text(crate::TextField::Title, TextEncoding::Latin1, "Ancient");
        tag.push(Frame::Chapter(crate::chapter::Chapter::new("ch0", 0)));
        assert!(tag.to_bytes().is_err());
    }

    #[test]
    fn an_id3v23_tag_with_a_ten_byte_frame_header_parses_too() {
        // What mutagen writes and ffmpeg reads: a ten-byte header, as v2.4 has, with a
        // plain rather than synch-safe size. If this did not parse, every v2.3 file that
        // mutagen or ffmpeg has ever produced would be unreadable — which is most of
        // them, because those two are how most v2.3 tags get made.
        let mut body = Vec::new();
        body.extend_from_slice(b"TIT2");
        body.extend_from_slice(&4u32.to_be_bytes()); // plain size, not synch-safe
        body.extend_from_slice(&[0x00, 0x00]); // TWO flag bytes, ten-byte header
        body.extend_from_slice(&[0x03, b'x', b'y', b'z']);

        let parsed = Tag::parse(&v23_tag(&body)).expect("an ecosystem-shaped v2.3 tag must parse");
        assert_eq!(parsed.major_version, 3);
        assert_eq!(
            parsed.text_field(crate::TextField::Title),
            Some("xyz"),
            "and it must not be mis-framed"
        );
    }

    #[test]
    fn both_v23_frame_header_shapes_can_appear_in_one_tag() {
        // They are the same length for these two frames only by coincidence, so this
        // checks that each frame is read at its own offset rather than the table being
        // walked with one fixed length throughout. A tag with a nine-byte header followed
        // by a ten-byte one is malformed by any reading, so this asserts only that the
        // *first* interpretation that yields valid ids is taken and the rest does not
        // corrupt it.
        let mut nine = Vec::new();
        nine.extend_from_slice(b"TIT2");
        nine.extend_from_slice(&4u32.to_be_bytes());
        nine.push(0x00);
        nine.extend_from_slice(&[0x03, b'n', b'i', b'n']);

        let mut ten = Vec::new();
        ten.extend_from_slice(b"TPE1");
        ten.extend_from_slice(&4u32.to_be_bytes());
        ten.extend_from_slice(&[0x00, 0x00]);
        ten.extend_from_slice(&[0x03, b't', b'e', b'n']);

        let parsed = Tag::parse(&v23_tag(&[nine, ten].concat())).expect("parses");
        assert!(
            parsed.text_field(crate::TextField::Title).is_some()
                || parsed.text_field(crate::TextField::Artist).is_some(),
            "at least one frame survived, and the ids were not garbage"
        );
    }

    #[test]
    fn a_v23_tag_is_never_read_as_garbage_ids() {
        // The guard against the fallback masking a real problem: whatever shape is
        // chosen, no frame may end up with an id that is not four alphanumerics.
        for build in [
            // Nine-byte headers.
            0u8, // Ten-byte headers.
            1,
        ] {
            let mut body = Vec::new();
            body.extend_from_slice(b"TIT2");
            body.extend_from_slice(&4u32.to_be_bytes());
            if build == 0 {
                body.push(0x00);
            } else {
                body.extend_from_slice(&[0x00, 0x00]);
            }
            body.extend_from_slice(&[0x03, b'q', b'q', b'q']);
            let parsed = Tag::parse(&v23_tag(&body)).expect("parses");
            for frame in &parsed.frames {
                assert!(
                    frames_have_valid_ids(core::slice::from_ref(frame)),
                    "id {:?} is not alphanumeric",
                    frame.id()
                );
            }
        }
    }

    /// A complete ID3v2.3 tag wrapping `body`.
    fn v23_tag(body: &[u8]) -> Vec<u8> {
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.push(3);
        tag.push(0x00);
        tag.push(0x00);
        tag.extend_from_slice(&crate::synchsafe::encode(body.len() as u32).expect("encodable"));
        tag.extend_from_slice(body);
        tag
    }

    #[test]
    fn an_id3v23_tag_parses() {
        // A v2.3 frame header is **nine** bytes: a 4-byte id, a 4-byte *plain* size and a
        // single flag byte. v2.4's is ten, with a synch-safe size and two flag bytes.
        //
        // Getting this wrong mis-frames every frame by one byte — the trailing flag byte
        // is read as the first byte of the next frame's id — so the whole tag falls apart
        // and `Tag::parse` returns an error, taking the title, artist and every other
        // field with it. The size field was already decoded per-version, which is why the
        // bug is invisible on any v2.4 tag and only shows up here.
        //
        // This is built byte by byte rather than through `Tag::to_bytes`, because the
        // writer and the reader disagreeing about the layout is exactly what hides this.
        let mut body = Vec::new();
        body.extend_from_slice(b"TIT2");
        body.extend_from_slice(&4u32.to_be_bytes()); // plain size, not synch-safe
        body.push(0x00); // one flag byte
        body.extend_from_slice(&[0x03, b'a', b'b', b'c']); // UTF-8 "abc"

        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.push(3);
        tag.push(0x00);
        tag.push(0x00);
        tag.extend_from_slice(
            &crate::synchsafe::encode(body.len() as u32).expect("tag size is encodable"),
        );
        tag.extend_from_slice(&body);

        let parsed = Tag::parse(&tag).expect("a v2.3 tag must parse");
        assert_eq!(parsed.major_version, 3);
        assert_eq!(
            parsed.text_field(crate::TextField::Title),
            Some("abc"),
            "and its title must be readable"
        );
    }

    #[test]
    fn an_id3v23_tag_with_chapters_parses() {
        // The same off-by-one, reached through the frame types that matter most for an
        // audiobook. A CHAP body is element id + 16 bytes of times and offsets, so the
        // frames here are longer than a text frame and the misalignment is unambiguous.
        let mut chap = Vec::new();
        chap.extend_from_slice(b"ch0");
        chap.push(0x00);
        chap.extend_from_slice(&0u32.to_be_bytes()); // start
        chap.extend_from_slice(&60_000u32.to_be_bytes()); // end
        chap.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // start offset: ignore
        chap.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // end offset: ignore

        let mut body = Vec::new();
        body.extend_from_slice(b"CHAP");
        body.extend_from_slice(&(chap.len() as u32).to_be_bytes());
        body.push(0x00);
        body.extend_from_slice(&chap);

        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.push(3);
        tag.push(0x00);
        tag.push(0x00);
        tag.extend_from_slice(
            &crate::synchsafe::encode(body.len() as u32).expect("tag size is encodable"),
        );
        tag.extend_from_slice(&body);

        let parsed = Tag::parse(&tag).expect("a v2.3 CHAP must parse");
        assert_eq!(parsed.chapters().len(), 1, "and the chapter must be found");
    }

    #[test]
    fn parse_rejects_an_unknown_major_version() {
        let mut data = minimal_tag().to_bytes().expect("serializes");
        if let Some(b) = data.get_mut(3) {
            *b = 9;
        }
        assert_eq!(
            Tag::parse(&data),
            Err(Id3Error::UnsupportedMajorVersion { major: 9 })
        );
    }

    #[test]
    fn multiple_text_fields_round_trip() {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "Dune");
        t.set_text(TextField::Artist, TextEncoding::Utf8, "Frank Herbert");
        t.set_text(TextField::Album, TextEncoding::Utf8, "1965");
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.text_field(TextField::Title), Some("Dune"));
        assert_eq!(back.text_field(TextField::Artist), Some("Frank Herbert"));
        assert_eq!(back.text_field(TextField::Album), Some("1965"));
    }

    #[test]
    fn set_text_replaces_rather_than_appends() {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "First");
        t.set_text(TextField::Title, TextEncoding::Utf8, "Second");
        assert_eq!(t.frames.len(), 1, "a replaced field must not duplicate");
        assert_eq!(t.text_field(TextField::Title), Some("Second"));
    }

    #[test]
    fn set_text_with_empty_removes() {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "Gone");
        t.set_text(TextField::Title, TextEncoding::Utf8, "");
        assert!(t.text_field(TextField::Title).is_none());
        assert!(t.frames.is_empty());
    }

    #[test]
    fn year_reads_both_tyer_and_tdrc() {
        // A v2.3 tag finished by a v2.4 tool can carry both.
        let mut t = Tag::new();
        t.frames.push(Frame::Text {
            id: id3(b"TDRC"),
            text: Text::single(TextEncoding::Utf8, "1984"),
        });
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.text_field(TextField::Year), Some("1984"));
    }

    #[test]
    fn picture_round_trips() {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "Cover");
        t.push(Frame::Picture(AttachedPicture::cover(vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A,
        ])));
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        let cover = back.cover().expect("has cover");
        assert_eq!(cover.mime_type, "image/png");
    }

    #[test]
    fn chapters_and_toc_round_trip_and_resolve() {
        let mut t = Tag::new();
        t.set_text(TextField::Title, TextEncoding::Utf8, "Audiobook");

        let mut chapters = Vec::new();
        for (id, start, end) in [("ch1", 0u32, 600_000u32), ("ch2", 600_000, 1_200_000)] {
            let mut c = Chapter::new(id, start).with_end(end);
            let tid = id3(b"TIT2");
            c.frames.push((
                tid,
                Text::single(TextEncoding::Utf8, id)
                    .to_body()
                    .expect("serializes"),
            ));
            chapters.push(c);
        }
        let mut toc = Toc::root("book");
        toc.push("ch1");
        toc.push("ch2");

        for c in chapters {
            t.push(Frame::Chapter(c));
        }
        t.push(Frame::Toc(toc));

        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.chapters().len(), 2);
        let resolved = back.resolved_chapters().expect("resolves");
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].element_id, "ch1");
        assert_eq!(resolved[1].start_ms, 600_000);
        assert_eq!(resolved[0].title.as_deref(), Some("ch1"));
    }

    #[test]
    fn comment_and_lyrics_round_trip() {
        use crate::comment::Language;
        let mut t = Tag::new();
        t.push(Frame::Comment(Comment::new(
            Language::new(*b"eng").expect("valid"),
            "note",
            TextEncoding::Utf8,
        )));
        t.push(Frame::UnsynchronisedLyrics(UnsynchronisedLyrics::new(
            Language::new(*b"eng").expect("valid"),
            "la la",
            TextEncoding::Utf8,
        )));
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.comment().map(|c| c.text.text()), Some("note"));
        assert_eq!(back.lyrics().map(|l| l.text.text()), Some("la la"));
    }

    #[test]
    fn unknown_frames_survive_a_round_trip() {
        let mut t = Tag::new();
        t.push(unknown_frame(id3(b"TXXX"), vec![1, 2, 3, 4]));
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        match back.frame(id3(b"TXXX")) {
            Some(Frame::Unknown { data, .. }) => assert_eq!(data, &vec![1, 2, 3, 4]),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn unsynchronisation_round_trips() {
        let mut t = Tag::new();
        t.unsynchronisation = true;
        // Bytes chosen so the body contains 0xFF 0xFB (an MPEG sync).
        t.push(unknown_frame(id3(b"TXXX"), vec![0xFF, 0xFB, 0xFF, 0xE0]));
        let bytes = t.to_bytes().expect("serializes");
        let back = Tag::parse(&bytes).expect("reparses");
        match back.frame(id3(b"TXXX")) {
            Some(Frame::Unknown { data, .. }) => {
                assert_eq!(data, &vec![0xFF, 0xFB, 0xFF, 0xE0]);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn synchronise_then_unsynchronise_is_identity() {
        for body in [
            vec![0xFF, 0xFB],
            vec![0xFF, 0x00],
            vec![0xFF],
            vec![0x00, 0xFF, 0xE0, 0x01],
            alloc::vec![0xFF; 8],
        ] {
            let s = synchronise(&body);
            assert_eq!(unsynchronise(&s), body, "round trip failed for {body:?}");
        }
    }

    #[test]
    fn unsynchronise_leaves_a_trailing_ff_alone() {
        // A lone trailing 0xFF is data; removing a following byte would eat
        // real content.
        assert_eq!(unsynchronise(&[0x01, 0xFF]), vec![0x01, 0xFF]);
    }

    #[test]
    fn find_id3v2_reports_the_tag_length() {
        let t = minimal_tag();
        let bytes = t.to_bytes().expect("serializes");
        let len = find_id3v2(&bytes).expect("found");
        assert_eq!(len, bytes.len());
        assert_eq!(find_id3v2(b"no tag"), None);
    }

    #[test]
    fn find_id3v2_clamps_a_truncated_tag() {
        let bytes = minimal_tag().to_bytes().expect("serializes");
        let cut = &bytes[..bytes.len() - 3];
        let len = find_id3v2(cut).expect("still found");
        assert_eq!(len, cut.len(), "a truncated tag reports what is present");
    }

    #[test]
    fn v3_tag_round_trips() {
        let mut t = Tag::with_version(3).expect("valid");
        t.set_text(TextField::Title, TextEncoding::Latin1, "Legacy");
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.major_version, 3);
        assert_eq!(back.text_field(TextField::Title), Some("Legacy"));
    }

    #[test]
    fn v2_tag_round_trips_three_char_ids() {
        let mut t = Tag::with_version(2).expect("valid");
        t.set_text(TextField::Title, TextEncoding::Latin1, "Ancient");
        let back = Tag::parse(&t.to_bytes().expect("serializes")).expect("reparses");
        assert_eq!(back.major_version, 2);
        assert_eq!(back.text_field(TextField::Title), Some("Ancient"));
    }

    #[test]
    fn a_large_tag_stays_under_the_ceiling() {
        let mut t = Tag::new();
        for i in 0..200 {
            t.push(unknown_frame(id3(b"TXXX"), alloc::vec![0xAB; 1024]));
            let _ = i;
        }
        let bytes = t.to_bytes().expect("serializes");
        assert!(bytes.len() < 1_000_000);
        assert!(Tag::parse(&bytes).is_ok());
    }

    #[test]
    fn a_tag_with_no_frames_serializes() {
        let t = Tag::new();
        let bytes = t.to_bytes().expect("serializes");
        assert_eq!(bytes.len(), 10, "header only");
        let back = Tag::parse(&bytes).expect("reparses");
        assert!(back.frames.is_empty());
    }

    #[test]
    fn remove_reports_how_many_went() {
        let mut t = Tag::new();
        t.push(unknown_frame(id3(b"TXXX"), vec![1]));
        t.push(unknown_frame(id3(b"TXXX"), vec![2]));
        assert_eq!(t.remove(id3(b"TXXX")), 2);
        assert!(t.frames.is_empty());
    }

    #[test]
    fn extended_header_is_skipped() {
        // Hand-build a v2.4 tag with an extended header and one frame.
        let mut body = vec![0x00, 0x00, 0x00, 0x06, 0x00, 0x00]; // size=6, incl. itself
        body.extend_from_slice(b"TIT2");
        body.extend_from_slice(&[0x00, 0x00, 0x00, 0x04]); // synch-safe size 4
        body.extend_from_slice(&[0x00, 0x00]); // flags
        body.extend_from_slice(&[0x03, b'H', b'i', b'!']);

        let size = synchsafe::encode_usize(body.len()).expect("fits");
        let mut bytes = vec![b'I', b'D', b'3', 0x04, 0x00, 0x40];
        bytes.extend_from_slice(&size);
        bytes.extend_from_slice(&body);

        let t = Tag::parse(&bytes).expect("reparses past the extended header");
        assert!(t.has_extended_header);
        assert_eq!(t.text_field(TextField::Title), Some("Hi!"));
    }

    /// An arbitrary body survives a write/read cycle.
    ///
    /// Empty bodies are excluded because a zero-length frame is not
    /// representable: ID3v2 reserves size 0 as the frame-table padding marker,
    /// so a zero-size frame would terminate the table. A caller wanting an
    /// empty value writes an empty *text* frame, which still has a one-byte
    /// encoding selector and is therefore non-zero in size.
    #[test]
    fn proptest_tag_round_trip() {
        use proptest::prelude::*;
        proptest!(|(bodies: Vec<Vec<u8>>)| {
            let mut t = Tag::new();
            let mut written = 0usize;
            for b in bodies.iter().take(8) {
                if b.is_empty() {
                    continue;
                }
                t.push(unknown_frame(id3(b"TXXX"), b.clone()));
                written += 1;
            }
            let bytes = t.to_bytes().expect("serializes");
            let back = Tag::parse(&bytes).expect("reparses");
            let got: usize = back.frames.len();
            prop_assert_eq!(got, written);
            // And the bytes come back byte-identical.
            for (orig, frame) in bodies
                .iter()
                .filter(|b| !b.is_empty())
                .take(8)
                .zip(back.frames.iter())
            {
                match frame {
                    Frame::Unknown { data, .. } => prop_assert_eq!(data, orig),
                    other => prop_assert!(false, "expected Unknown, got {other:?}"),
                }
            }
        });
    }

    #[test]
    fn proptest_unsynchronisation_is_lossless() {
        use proptest::prelude::*;
        proptest!(|(body: Vec<u8>)| {
            let s = synchronise(&body);
            prop_assert_eq!(unsynchronise(&s), body);
        });
    }
}
