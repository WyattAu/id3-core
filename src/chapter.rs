//! Chapters (`CHAP`) and tables of contents (`CTOC`).
//!
//! This is the load-bearing pair for an audiobook library: `CHAP` marks a
//! chapter's element id, start time, and end time in milliseconds, and `CTOC`
//! nests chapters into parts. Together they are how a 40-hour book becomes
//! addressable without a separate sidecar file.
//!
//! # Element ids, and why they are strings
//!
//! Both frames key on a null-terminated *string* element id, not an integer.
//! That is deliberate in the spec: an id space shared with `TXXX` user-defined
//! frames lets a chapter reference an attached picture or a URL, so a player
//! can render per-chapter art. Ids are therefore opaque and this crate never
//! interprets them — it only guarantees they round-trip.
//!
//! # Time
//!
//! Offsets are unsigned milliseconds from the start of the audio. The spec has
//! no negative or "unknown end" encoding, so [`Chapter::end_ms`] is `None` for
//! an open-ended chapter — which real files do contain, and which this crate
//! round-trips rather than rejecting.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::Id3Error;

/// A single chapter (`CHAP`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// The element id, matched by a `CTOC`'s entries. Opaque to this crate.
    pub element_id: String,
    /// Start offset in milliseconds from the beginning of the audio.
    pub start_ms: u32,
    /// End offset in milliseconds, or `None` for an open-ended chapter.
    pub end_ms: Option<u32>,
    /// Byte offsets into the tag where this frame starts and ends.
    ///
    /// The spec defines these as "a pointer to the start/end of the chapter in
    /// the tag", used by `CTOC` to order nested frames. Zero when unknown.
    pub start_offset: u32,
    /// See [`Chapter::start_offset`].
    pub end_offset: u32,
    /// Sub-frames attached to this chapter: `TIT2` for a title, `TIT3` for a
    /// subtitle, `APIC` for art, `WXXX` for a URL.
    pub frames: Vec<(crate::FrameId, Vec<u8>)>,
}

/// Encode a `TIT2` sub-frame for a chapter or TOC heading.
///
/// Returns `None` rather than a partially built frame when the id cannot be formed,
/// because a sub-frame with an invalid id is worse than no sub-frame: it would be written
/// out and then skipped by every reader.
fn title_subframe(text: &str) -> Option<(crate::FrameId, Vec<u8>)> {
    let id = crate::FrameId::new(*b"TIT2").ok()?;
    let body = crate::text::Text::single(crate::text::TextEncoding::Utf8, text)
        .to_body()
        .ok()?;
    Some((id, body))
}

impl Chapter {
    /// A chapter with just an id and a start time; the common case.
    #[must_use]
    pub fn new(element_id: &str, start_ms: u32) -> Self {
        Self {
            element_id: String::from(element_id),
            start_ms,
            end_ms: None,
            start_offset: 0,
            end_offset: 0,
            frames: Vec::new(),
        }
    }

    /// Set the end offset, closing the chapter.
    #[must_use]
    pub fn with_end(mut self, end_ms: u32) -> Self {
        self.end_ms = Some(end_ms);
        self
    }

    /// The chapter's duration in milliseconds, when both ends are known.
    #[must_use]
    pub fn duration_ms(&self) -> Option<u32> {
        self.end_ms.map(|end| end.saturating_sub(self.start_ms))
    }

    /// Attach a `TIT2` title to this chapter.
    ///
    /// Reading a chapter's title was public but writing one was not: the sub-frame list
    /// is a pair of an id and a raw body, so a caller wanting to *emit* chapters had to
    /// hand-encode a text frame to get a title into a file. That is workable but it means
    /// the encoding choice — and the trailing NUL a real writer emits — is left to
    /// whoever is holding the bytes, which is how the two ends drift apart.
    ///
    /// UTF-8 encoding is used, which is what mutagen, ffmpeg and this crate's own reader
    /// all produce, and the sub-frame is terminated with a NUL as ID3v2.4 requires.
    /// Replaces any existing `TIT2`, so calling it twice does not accumulate frames.
    #[must_use]
    pub fn with_title(mut self, title: &str) -> Self {
        self.set_title(title);
        self
    }

    /// Set or clear the `TIT2` title on this chapter.
    ///
    /// An empty title clears the sub-frame rather than writing an empty one, because a
    /// `TIT2` with nothing in it is read back as a present-but-blank title, which is not
    /// the same as no title.
    pub fn set_title(&mut self, title: &str) {
        self.remove_subframe(b"TIT2");
        if title.is_empty() {
            return;
        }
        if let Some(frame) = title_subframe(title) {
            self.frames.push(frame);
        }
    }

    /// This chapter's `TIT2` title, if it has one.
    ///
    /// Owned rather than borrowed because decoding a text frame produces a fresh
    /// `String`, and returning a reference to it would either borrow the frame body —
    /// which holds the *encoded* bytes, not the decoded text — or leak it.
    #[must_use]
    pub fn title(&self) -> Option<String> {
        let body = self.subframe(b"TIT2")?;
        let id = crate::FrameId::new(*b"TIT2").ok()?;
        crate::text::Text::parse(id, body)
            .ok()
            .map(|t| String::from(t.text()))
    }

    /// Drop any sub-frame with this id.
    pub fn remove_subframe(&mut self, id: &[u8; 4]) {
        self.frames
            .retain(|(frame_id, _)| frame_id.as_bytes() != id);
    }

    /// The first sub-frame with this id.
    #[must_use]
    pub fn subframe(&self, id: &[u8; 4]) -> Option<&[u8]> {
        self.frames
            .iter()
            .find(|(frame_id, _)| frame_id.as_bytes() == id)
            .map(|(_, body)| body.as_slice())
    }

    /// Parse a `CHAP` body.
    ///
    /// # Errors
    ///
    /// [`Id3Error::UnterminatedElementId`] if the element id is unterminated,
    /// [`Id3Error::TruncatedFrame`] if the fixed header is short, and
    /// [`Id3Error::SizeTooLarge`] if a sub-frame runs past the body.
    pub fn parse(body: &[u8]) -> Result<Self, Id3Error> {
        let id_end = body
            .iter()
            .position(|b| *b == 0x00)
            .ok_or(Id3Error::UnterminatedElementId)?;
        let element_id = core::str::from_utf8(body.get(..id_end).unwrap_or(&[]))
            .map_err(|_| Id3Error::InvalidUtf8)?;
        let element_id = String::from(element_id);

        // element id NUL | 4-byte start | 4-byte end | 4-byte start offset |
        // 4-byte end offset | sub-frames
        let fixed = body.get(id_end + 1..).ok_or(Id3Error::TruncatedFrame {
            id: id3(b"CHAP"),
            expected: 16,
            got: 0,
        })?;

        if fixed.len() < 16 {
            return Err(Id3Error::TruncatedFrame {
                id: id3(b"CHAP"),
                expected: 16,
                got: fixed.len(),
            });
        }
        let start_ms = be32(fixed, 0);
        let end_ms = be32(fixed, 4);
        let start_offset = be32(fixed, 8);
        let end_offset = be32(fixed, 12);

        let frames = parse_subframes(fixed.get(16..).unwrap_or(&[]))?;

        // The spec encodes an unused end time as 0xFFFFFFFF.
        let end_ms = if end_ms == u32::MAX {
            None
        } else {
            Some(end_ms)
        };

        Ok(Self {
            element_id,
            start_ms,
            end_ms,
            start_offset,
            end_offset,
            frames,
        })
    }

    /// Serialize to a `CHAP` body.
    #[must_use]
    pub fn to_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(self.element_id.as_bytes());
        out.push(0x00);
        push_be32(&mut out, self.start_ms);
        push_be32(&mut out, self.end_ms.unwrap_or(u32::MAX));
        push_be32(&mut out, self.start_offset);
        push_be32(&mut out, self.end_offset);
        for (id, data) in &self.frames {
            out.extend_from_slice(id.as_bytes());
            let size = data.len() as u32;
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&[0x00, 0x00]); // frame flags
            out.extend_from_slice(data);
        }
        out
    }
}

/// A table of contents (`CTOC`), nesting chapters and other TOCs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toc {
    /// This TOC's own element id.
    pub element_id: String,
    /// Whether players should show the entries as a flat list (`0x01`) or a
    /// nested tree (`0x00`).
    pub top_level: bool,
    /// Whether this TOC is the root of the hierarchy.
    pub ordered: bool,
    /// Child element ids, in order. May name chapters or other TOCs.
    pub entries: Vec<String>,
    /// Sub-frames: typically `TIT2` for a heading.
    pub frames: Vec<(crate::FrameId, Vec<u8>)>,
}

impl Toc {
    /// A top-level, ordered TOC with the given id.
    #[must_use]
    pub fn root(element_id: &str) -> Self {
        Self {
            element_id: String::from(element_id),
            top_level: true,
            ordered: true,
            entries: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// Add a child element id.
    pub fn push(&mut self, element_id: &str) {
        self.entries.push(String::from(element_id));
    }

    /// Parse a `CTOC` body.
    ///
    /// # Errors
    ///
    /// As [`Chapter::parse`], plus a child count that cannot fit the body.
    pub fn parse(body: &[u8]) -> Result<Self, Id3Error> {
        let id_end = body
            .iter()
            .position(|b| *b == 0x00)
            .ok_or(Id3Error::UnterminatedElementId)?;
        let element_id = core::str::from_utf8(body.get(..id_end).unwrap_or(&[]))
            .map_err(|_| Id3Error::InvalidUtf8)?;
        let element_id = String::from(element_id);

        // element id NUL | flags | entry count | child ids | sub-frames
        let header = body.get(id_end + 1..).ok_or(Id3Error::TruncatedFrame {
            id: id3(b"CTOC"),
            expected: 2,
            got: 0,
        })?;

        let (flags, count) = match (header.first(), header.get(1)) {
            (Some(f), Some(c)) => (*f, *c),
            _ => {
                return Err(Id3Error::TruncatedFrame {
                    id: id3(b"CTOC"),
                    expected: 2,
                    got: header.len(),
                })
            }
        };
        // Top-level is bit 1 and ordered is bit 0.
        //
        // The spec draws this as `%000000ab`, which reads as "six zero bits, then a,
        // then b" — a is bit 1 and b is bit 0, counting from the *low* end. Reading it
        // as a = bit 7 transposes the two flags, and it is tempting because the two
        // disagree often enough to look plausible: a TOC that is top level and not
        // ordered comes back as the exact opposite, so a caller looking for the root
        // finds nothing and every consumer loses the chapter order.
        //
        // Established from two implementations that share no code: mutagen's
        // `CTOCFlags.TOP_LEVEL = 0x2`, `ORDERED = 0x1`, and TagLib's
        // `d->isTopLevel = data & 2; d->isOrdered = data & 1`. A file mutagen wrote with
        // `flags=CTOCFlags.TOP_LEVEL` carries `0x02` on the wire.
        let top_level = flags & 0x02 != 0;
        let ordered = flags & 0x01 != 0;

        let mut cursor = header.get(2..).unwrap_or(&[]);
        let mut entries = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let nul = cursor
                .iter()
                .position(|b| *b == 0x00)
                .ok_or(Id3Error::UnterminatedElementId)?;
            let entry = core::str::from_utf8(cursor.get(..nul).unwrap_or(&[]))
                .map_err(|_| Id3Error::InvalidUtf8)?;
            entries.push(String::from(entry));
            cursor = cursor.get(nul + 1..).unwrap_or(&[]);
        }

        let frames = parse_subframes(cursor)?;

        Ok(Self {
            element_id,
            top_level,
            ordered,
            entries,
            frames,
        })
    }

    /// Serialize to a `CTOC` body.
    ///
    /// # Panics
    ///
    /// Never: a count above 255 is truncated to the spec's single-byte field,
    /// matching what a reader would see anyway.
    #[must_use]
    pub fn to_body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(self.element_id.as_bytes());
        out.push(0x00);
        let mut flags = 0x00u8;
        // The same layout [`Toc::parse`] reads, so a TOC written here is read back
        // identically by mutagen and TagLib. Getting this wrong writes a frame every
        // other implementation calls "not top level", which loses the chapter order
        // rather than merely setting the wrong flag.
        if self.top_level {
            flags |= 0x02;
        }
        if self.ordered {
            flags |= 0x01;
        }
        out.push(flags);
        // The count is a single byte in the spec. Saturate rather than panic.
        let count = self.entries.len().min(usize::from(u8::MAX));
        out.push(count as u8);
        for entry in self.entries.iter().take(count) {
            out.extend_from_slice(entry.as_bytes());
            out.push(0x00);
        }
        for (id, data) in &self.frames {
            out.extend_from_slice(id.as_bytes());
            let size = data.len() as u32;
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&[0x00, 0x00]);
            out.extend_from_slice(data);
        }
        out
    }
}

/// A chapter and its place in a table of contents, resolved.
///
/// This is the shape a player wants: chapters in playing order with their
/// titles already attached, which is more than the raw `CHAP`/`CTOC` pair
/// provides — the link between them is by element id and nothing on the wire
/// enforces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChapter {
    /// The chapter's element id.
    pub element_id: String,
    /// Start offset in milliseconds.
    pub start_ms: u32,
    /// End offset, when known.
    pub end_ms: Option<u32>,
    /// The chapter's `TIT2` title, if a sub-frame provided one.
    pub title: Option<String>,
    /// Depth in the TOC tree: 0 for a chapter named directly by the root TOC.
    pub depth: usize,
}

impl ResolvedChapter {
    /// Duration in milliseconds, when both ends are known.
    #[must_use]
    pub fn duration_ms(&self) -> Option<u32> {
        self.end_ms.map(|e| e.saturating_sub(self.start_ms))
    }
}

/// Resolve `CHAP` frames against a `CTOC` into playing order.
///
/// Chapters are returned sorted by `start_ms`, because the TOC order and the
/// temporal order disagree in real files and playback cares about time. Each
/// chapter's depth reflects how deeply it was nested, so a player can indent.
///
/// Chapters not named by any TOC are still returned, at depth 0 — losing them
/// would silently drop content from a library index.
#[must_use]
pub fn resolve<'a>(chapters: &'a [Chapter], root: Option<&'a Toc>) -> Vec<ResolvedChapter> {
    let mut depths: Vec<(String, usize)> = Vec::new();
    if let Some(t) = root {
        walk_toc(t, 0, &mut depths, chapters);
    }

    let mut out: Vec<ResolvedChapter> = chapters
        .iter()
        .map(|c| {
            let depth = depths
                .iter()
                .find(|(id, _)| *id == c.element_id)
                .map_or(0, |(_, d)| *d);
            ResolvedChapter {
                element_id: c.element_id.clone(),
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                // The same decoding as `Chapter::title`, so a title is read identically
                // whether a caller resolves a TOC or asks a single chapter.
                title: c.title(),
                depth,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        a.start_ms
            .cmp(&b.start_ms)
            .then_with(|| a.element_id.cmp(&b.element_id))
    });
    out
}

/// Walk a TOC, assigning depth to each element id it names.
fn walk_toc(toc: &Toc, depth: usize, acc: &mut Vec<(String, usize)>, chapters: &[Chapter]) {
    for entry in &toc.entries {
        acc.push((entry.clone(), depth));
        // A child id naming another TOC nests further.
        if !chapters.iter().any(|c| c.element_id == *entry) {
            // Depth beyond the chapters we hold cannot be resolved without the
            // other TOCs; stop rather than guess.
            continue;
        }
    }
    // Nested TOCs are stored as sub-frames on this one, so recurse into any
    // CTOC sub-frame.
    for (id, data) in &toc.frames {
        if id.as_bytes() == b"CTOC" {
            if let Ok(child) = Toc::parse(data) {
                walk_toc(&child, depth + 1, acc, chapters);
            }
        }
    }
}

/// Read a big-endian `u32` at `offset`, or 0 if out of bounds.
fn be32(data: &[u8], offset: usize) -> u32 {
    let b = data.get(offset..offset + 4);
    match b {
        Some([a, b2, c, d]) => u32::from_be_bytes([*a, *b2, *c, *d]),
        _ => 0,
    }
}

fn push_be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

const fn id3(s: &[u8; 4]) -> crate::FrameId {
    crate::FrameId::raw(s)
}

/// Parse the sub-frame list trailing a `CHAP`/`CTOC`.
///
/// Sub-frames use the v2.4 header shape (id, 4-byte size, 2 flag bytes)
/// regardless of the parent frame's version, which is what the spec says and
/// what every real encoder emits.
fn parse_subframes(mut data: &[u8]) -> Result<Vec<(crate::FrameId, Vec<u8>)>, Id3Error> {
    let mut out = Vec::new();
    while let Some(head) = data.get(..10) {
        let id_bytes: [u8; 4] = match head.get(..4).and_then(|s| s.try_into().ok()) {
            Some(b) => b,
            None => break,
        };
        // A sub-frame id of all NULs marks the end of the list in some encoders.
        if id_bytes == [0u8; 4] {
            break;
        }
        let id = crate::FrameId::raw(&id_bytes);
        let size = be32(data, 4);
        let body = data.get(10..).unwrap_or(&[]);
        let size_usize = match usize::try_from(size) {
            Ok(v) => v,
            Err(_) => {
                return Err(Id3Error::SizeTooLarge {
                    declared: size,
                    available: body.len(),
                })
            }
        };
        if body.len() < size_usize {
            return Err(Id3Error::TruncatedFrame {
                id,
                expected: size_usize,
                got: body.len(),
            });
        }
        out.push((id, body.get(..size_usize).unwrap_or(&[]).to_vec()));
        data = body.get(size_usize..).unwrap_or(&[]);
    }
    Ok(out)
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

    fn title_frame(text: &str) -> (crate::FrameId, Vec<u8>) {
        let id = crate::FrameId::new(*b"TIT2").expect("valid");
        let body = crate::text::Text::single(crate::text::TextEncoding::Utf8, text)
            .to_body()
            .expect("serializes");
        (id, body)
    }

    #[test]
    fn chapter_round_trips() {
        let c = Chapter::new("ch1", 0).with_end(30_000);
        let body = c.to_body();
        let back = Chapter::parse(&body).expect("reparses");
        assert_eq!(back.element_id, "ch1");
        assert_eq!(back.start_ms, 0);
        assert_eq!(back.end_ms, Some(30_000));
        assert_eq!(back.duration_ms(), Some(30_000));
    }

    #[test]
    fn chapter_with_no_end_round_trips_as_none() {
        // 0xFFFFFFFF is the spec's "unset" marker.
        let c = Chapter::new("open", 5_000);
        assert_eq!(c.end_ms, None);
        let back = Chapter::parse(&c.to_body()).expect("reparses");
        assert_eq!(
            back.end_ms, None,
            "unset end must stay None, not Some(0xFFFFFFFF)"
        );
        assert_eq!(back.duration_ms(), None);
    }

    #[test]
    fn chapter_carries_a_title_subframe() {
        let mut c = Chapter::new("ch1", 0).with_end(1_000);
        c.frames.push(title_frame("Chapter One"));
        let back = Chapter::parse(&c.to_body()).expect("reparses");
        assert_eq!(back.frames.len(), 1);
        let resolved = resolve(&[back], None);
        assert_eq!(resolved[0].title.as_deref(), Some("Chapter One"));
    }

    #[test]
    fn chapter_rejects_an_unterminated_id() {
        let body = vec![b'a', b'b', b'c']; // no NUL
        assert_eq!(Chapter::parse(&body), Err(Id3Error::UnterminatedElementId));
    }

    #[test]
    fn chapter_rejects_a_short_header() {
        let mut body = vec![b'x', 0x00];
        body.extend_from_slice(&[0x00, 0x00, 0x00]); // only 3 of 16 header bytes
        assert!(matches!(
            Chapter::parse(&body),
            Err(Id3Error::TruncatedFrame { .. })
        ));
    }

    #[test]
    fn toc_round_trips() {
        let mut t = Toc::root("toc");
        t.push("ch1");
        t.push("ch2");
        let back = Toc::parse(&t.to_body()).expect("reparses");
        assert_eq!(back.element_id, "toc");
        assert!(back.top_level);
        assert!(back.ordered);
        assert_eq!(back.entries, vec![String::from("ch1"), String::from("ch2")]);
    }

    /// A top-level, **not** ordered TOC exactly as mutagen wrote one.
    ///
    /// Captured from `mutagen.id3.CTOC(element_id="toc", flags=CTOCFlags.TOP_LEVEL,
    /// child_element_ids=["ch0","ch1"])` saved as ID3v2.4. The flags byte on the wire is
    /// `0x02`, which is the discriminator: mutagen's `CTOCFlags.TOP_LEVEL` is `0x2` and
    /// its `ORDERED` is `0x1`, and TagLib independently agrees
    /// (`d->isTopLevel = data & 2; d->isOrdered = data & 1`).
    const MUTAGEN_TOP_LEVEL_NOT_ORDERED: &[u8] = &[
        b't', b'o', b'c', 0x00, // element id
        0x02, // flags: TOP_LEVEL set, ORDERED clear
        0x02, // entry count
        b'c', b'h', b'0', 0x00, b'c', b'h', b'1', 0x00,
    ];

    #[test]
    fn a_chapter_title_round_trips_through_the_wire() {
        // The sub-frame must survive the body serialisation, not merely sit in the struct:
        // `parse` reads sub-frames back out of the bytes, so a title that only existed in
        // memory would look fine until it was written.
        let c = Chapter::new("ch1", 0)
            .with_end(30_000)
            .with_title("Chapter One");
        let back = Chapter::parse(&c.to_body()).expect("reparses");
        assert_eq!(back.title().as_deref(), Some("Chapter One"));
        assert!(
            c.subframe(b"TIT2").is_some(),
            "the sub-frame is really there"
        );
    }

    #[test]
    fn a_chapter_title_handles_the_things_real_chapter_titles_contain() {
        for title in [
            "Plain",
            "Has = equals ; semi # hash",
            "Has \\ backslash",
            "Line one\nLine two",
            " Leading and trailing ",
            "Unicode: éè 中文 📚",
            "He said \"hi\"",
        ] {
            let c = Chapter::new("ch", 0).with_title(title);
            let back = Chapter::parse(&c.to_body()).expect("reparses");
            assert_eq!(back.title().as_deref(), Some(title), "for {title:?}");
        }
    }

    #[test]
    fn setting_a_title_twice_replaces_rather_than_accumulates() {
        // Two TIT2 sub-frames read back as whichever the reader happens to find first, so
        // an accumulating setter produces a chapter whose title depends on the reader.
        let c = Chapter::new("ch", 0)
            .with_title("First")
            .with_title("Second");
        assert_eq!(
            c.frames
                .iter()
                .filter(|(id, _)| id.as_bytes() == b"TIT2")
                .count(),
            1,
            "exactly one TIT2"
        );
        let back = Chapter::parse(&c.to_body()).expect("reparses");
        assert_eq!(back.title().as_deref(), Some("Second"));
    }

    #[test]
    fn an_empty_title_clears_the_subframe_rather_than_writing_a_blank_one() {
        // A TIT2 with nothing in it reads back as a present-but-blank title, which is not
        // the same as having no title at all.
        let mut c = Chapter::new("ch", 0).with_title("Something");
        c.set_title("");
        assert!(
            c.subframe(b"TIT2").is_none(),
            "no empty sub-frame is left behind"
        );
        assert_eq!(c.title(), None);
        assert_eq!(
            Chapter::parse(&c.to_body()).expect("reparses").title(),
            None
        );
    }

    #[test]
    fn a_chapter_with_no_title_reports_none_rather_than_empty() {
        assert_eq!(Chapter::new("ch", 0).title(), None);
    }

    #[test]
    fn top_level_is_bit_one_and_ordered_is_bit_zero() {
        // Reading these the other way round reports this TOC as *not* top level, so a
        // caller hunting for the root finds nothing and the chapters lose their order.
        let toc = Toc::parse(MUTAGEN_TOP_LEVEL_NOT_ORDERED).expect("parses");
        assert!(
            toc.top_level,
            "0x02 is TOP_LEVEL: mutagen and TagLib both put top-level at bit 1"
        );
        assert!(
            !toc.ordered,
            "and 0x02 has bit 0 clear, so this TOC is not ordered"
        );
    }

    #[test]
    fn an_ordered_but_not_top_level_toc_is_not_mistaken_for_the_root() {
        // The mirror image, which is what catches the swap the other way: `0x01` is
        // ORDERED alone. Reading bit 0 as top-level would hand a caller a non-root TOC
        // as the root and produce a wrong chapter order.
        let body = b"toc\x00\x01\x01ch0\x00";
        let toc = Toc::parse(body).expect("parses");
        assert!(toc.ordered);
        assert!(!toc.top_level, "0x01 is not the top-level flag");
    }

    #[test]
    fn our_written_flags_are_read_the_same_way_by_mutagen_and_taglib() {
        // The read side matters, but so does the write side: a CTOC this crate emits
        // with the flags transposed would be read by mutagen as *not* top level, so
        // every consumer would fail to find a root and lose the chapter order. Asserted
        // on the bytes rather than through our own parser, because our parser agreeing
        // with itself is the bug being fixed.
        // `Toc::root` is top level *and* ordered, so both flag bits are set: 0x02 for
        // top-level plus 0x01 for ordered. Asserting the bits separately is what makes
        // this a real check — asserting `0x03` alone would still pass with them swapped.
        let mut t = Toc::root("toc");
        t.push("ch0");
        let body = t.to_body();
        let nul = body
            .iter()
            .position(|b| *b == 0x00)
            .expect("has an element id");
        let flags = body.get(nul + 1).copied().expect("has a flags byte");
        assert_ne!(
            flags & 0x02,
            0,
            "top level must be bit 1, as mutagen and TagLib read it"
        );
        assert_ne!(flags & 0x01, 0, "ordered must be bit 0");

        // And the unambiguous case: top level with ordered clear is 0x02 alone, which is
        // the byte mutagen wrote and which the two bits cannot both explain.
        let mut only_top = Toc::root("toc");
        only_top.ordered = false;
        only_top.push("ch0");
        let body = only_top.to_body();
        let nul = body
            .iter()
            .position(|b| *b == 0x00)
            .expect("has an element id");
        assert_eq!(body.get(nul + 1), Some(&0x02), "the byte mutagen wrote");
        assert_eq!(
            body.get(nul + 2),
            Some(&0x01),
            "and the entry count follows"
        );
    }

    #[test]
    fn toc_flags_survive() {
        let t = Toc {
            element_id: String::from("x"),
            top_level: false,
            ordered: false,
            entries: Vec::new(),
            frames: Vec::new(),
        };
        let back = Toc::parse(&t.to_body()).expect("reparses");
        assert!(!back.top_level);
        assert!(!back.ordered);
    }

    #[test]
    fn toc_rejects_an_unterminated_child_id() {
        // header says 1 child, but no NUL follows.
        let mut body = vec![b't', 0x00, 0x01, 0x01];
        body.extend_from_slice(b"nope");
        assert!(matches!(
            Toc::parse(&body),
            Err(Id3Error::UnterminatedElementId)
        ));
    }

    #[test]
    fn resolve_orders_by_time_not_by_toc_order() {
        // The TOC lists them backwards; playback order must not follow it.
        let chapters = vec![
            Chapter::new("b", 60_000).with_end(90_000),
            Chapter::new("a", 0).with_end(30_000),
        ];
        let mut t = Toc::root("toc");
        t.push("b");
        t.push("a");
        let resolved = resolve(&chapters, Some(&t));
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].element_id, "a");
        assert_eq!(resolved[1].element_id, "b");
        assert_eq!(resolved[0].start_ms, 0);
    }

    #[test]
    fn resolve_keeps_chapters_the_toc_omits() {
        // Losing an unlisted chapter would silently drop it from an index.
        let chapters = vec![Chapter::new("orphan", 1_000)];
        let t = Toc::root("toc");
        let resolved = resolve(&chapters, Some(&t));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].element_id, "orphan");
    }

    #[test]
    fn resolve_without_a_toc_still_works() {
        let chapters = vec![Chapter::new("only", 0)];
        let resolved = resolve(&chapters, None);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].depth, 0);
    }

    #[test]
    fn resolve_is_stable_for_equal_start_times() {
        let chapters = vec![Chapter::new("z", 0), Chapter::new("a", 0)];
        let resolved = resolve(&chapters, None);
        assert_eq!(
            resolved[0].element_id, "a",
            "ties break on id for determinism"
        );
        assert_eq!(resolved[1].element_id, "z");
    }

    #[test]
    fn an_audiobook_shape_round_trips() {
        // Three chapters and a TOC — the minimum a player needs to navigate.
        let mut chapters = Vec::new();
        for (id, start, end, title) in [
            ("ch1", 0u32, 600_000u32, "Chapter 1"),
            ("ch2", 600_000, 1_500_000, "Chapter 2"),
            ("ch3", 1_500_000, 2_400_000, "Chapter 3"),
        ] {
            let mut c = Chapter::new(id, start).with_end(end);
            c.frames.push(title_frame(title));
            chapters.push(c);
        }
        let mut toc = Toc::root("book");
        toc.push("ch1");
        toc.push("ch2");
        toc.push("ch3");

        let parsed: Vec<Chapter> = chapters
            .iter()
            .map(|c| Chapter::parse(&c.to_body()).expect("chapter round trips"))
            .collect();
        let parsed_toc = Toc::parse(&toc.to_body()).expect("toc round trips");
        let resolved = resolve(&parsed, Some(&parsed_toc));

        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].title.as_deref(), Some("Chapter 1"));
        assert_eq!(resolved[2].end_ms, Some(2_400_000));
        assert_eq!(resolved[2].duration_ms(), Some(900_000));
        // Contiguous, non-overlapping chapters: the invariant a player assumes.
        for w in resolved.windows(2) {
            assert_eq!(w[0].end_ms, Some(w[1].start_ms));
        }
    }
}
