//! ID3v1 / ID3v2 audio tag reader and writer.
//!
//! `id3-core` is the estate's L0 metadata layer: the tag formats every audio
//! file carries, decoded and re-encoded without an allocation in the parse
//! path. It is `#![no_std]` + `alloc` with one runtime dependency (`thiserror`,
//! the same typed-error choice as `can-core`, `dbc-parse`, and `dsp-core`).
//!
//! # What it covers
//!
//! - **ID3v2.2 / v2.3 / v2.4**, including the three- to four-character frame
//!   id upgrade table, per-version frame-size encoding (plain big-endian in
//!   v2.3, synch-safe in v2.4), extended headers, and unsynchronisation.
//! - **ID3v1 / v1.1**, the 128-byte trailer, including the v1.1 track-number
//!   marker in the comment field.
//! - **Text frames** in all four encodings — Latin-1, BOM-marked UTF-16,
//!   UTF-16BE, UTF-8 — with Latin-1 decoded rather than rejected, because it is
//!   what the majority of tags in the wild still use.
//! - **`CHAP` / `CTOC`**, the chapter pair an audiobook library needs to make a
//!   40-hour book navigable, with [`chapter::resolve`] turning them into
//!   playing order.
//! - **`APIC`**, cover art, with MIME sniffing on the way in.
//! - **`COMM` / `USLT`**, comments and lyrics.
//!
//! Unknown frames are preserved verbatim as [`Frame::Unknown`] rather than
//! dropped, so a tag written by a newer encoder survives a read/write cycle.
//!
//! # What it deliberately does not do
//!
//! - **No image decoding.** `APIC` yields raw bytes and a sniffed MIME type;
//!   validating or transcoding an image belongs to `media-kit`, which owns that.
//! - **No `WAV`/`AIFF`/`MP4` container parsing.** This crate reads the tag
//!   format, not the file format. The chunk-level wrappers those formats need
//!   are separate concerns, and the estate's `mp4-core` covers the MP4 case.
//! - **No Vorbis comments.** Those are Ogg-container keys, a different spec
//!   with a different escaping rule.
//!
//! # Example
//!
//! Read a tag and find the title:
//!
//! ```
//! use id3_core::{Tag, TextField};
//!
//! let mut tag = Tag::new();
//! tag.set_text(TextField::Title, id3_core::TextEncoding::Utf8, "Dune");
//! tag.set_text(TextField::Artist, id3_core::TextEncoding::Utf8, "Frank Herbert");
//!
//! let bytes = tag.to_bytes().expect("serializes");
//! let parsed = Tag::parse(&bytes).expect("reparses");
//!
//! assert_eq!(parsed.text_field(TextField::Title), Some("Dune"));
//! assert_eq!(parsed.text_field(TextField::Artist), Some("Frank Herbert"));
//! ```
//!
//! Build an audiobook tag with chapters:
//!
//! ```
//! use id3_core::{Chapter, Frame, Tag, Toc};
//!
//! let mut tag = Tag::new();
//! let mut toc = Toc::root("book");
//!
//! for (id, start, end) in [("ch1", 0u32, 600_000u32), ("ch2", 600_000, 1_200_000)] {
//!     tag.push(Frame::Chapter(Chapter::new(id, start).with_end(end)));
//!     toc.push(id);
//! }
//! tag.push(Frame::Toc(toc));
//!
//! let parsed = Tag::parse(&tag.to_bytes().expect("serializes")).expect("reparses");
//! let chapters = parsed.resolved_chapters().expect("resolves");
//!
//! assert_eq!(chapters.len(), 2);
//! assert_eq!(chapters[1].start_ms, 600_000);
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

pub mod chapter;
pub mod comment;
pub mod error;
pub mod frame;
pub mod picture;
pub mod synchsafe;
pub mod tag;
pub mod text;
pub mod v1;

pub use chapter::{Chapter, ResolvedChapter, Toc};
pub use comment::{Comment, Language, UnsynchronisedLyrics};
pub use error::Id3Error;
pub use frame::{Frame, FrameId, TextField};
pub use picture::{AttachedPicture, PictureType};
pub use tag::{find_id3v2, Tag, ID3_IDENTIFIER, MAX_TAG_SIZE};
pub use text::{Text, TextEncoding};
pub use v1::{TagV1, ID3V1_LEN, ID3V1_MAGIC};
