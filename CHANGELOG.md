# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

### Fixed

- **Writing chapters into an ID3v2.2 tag silently lost them.** `CHAP` and `CTOC` do not
  exist in v2.2, which uses three-character frame ids and six-byte headers; the writer
  emitted the frames anyway with the id truncated to `CHA`, which no reader recognises as
  chapters. The write reported success and the file came back chapterless \u{2014} and it
  reparsed fine, which is what made the loss silent rather than obvious.

  ID3v2.2 now refuses with `ChaptersUnsupportedAtVersion`, whose message says to convert
  the tag to v2.3 or v2.4 first. Refusing everything also matters: a partial write would
  leave the tag modified in ways the caller did not ask for.

  Found by writing the same chapters at all three tag versions and reparsing, which is the
  kind of sweep that only happens once the question is "what does the estate trust itself
  for".

### Established about the format, by running the readers

- **UTF-8 chapter titles in an ID3v2.3 tag are read correctly by mutagen and ffprobe**,
  despite UTF-8 not being defined for v2.3 \u{2014} the earlier release note here said v2.3
  text must be Latin-1 or UTF-16, which is true for top-level frames and empirically too
  strong for frames nested inside `CHAP`, where both readers skip the version check the
  top-level path performs. The writer still emits UTF-8 there, and that is now a documented
  finding rather than an unnoticed assumption; a stricter reader may still reject it, which
  is the residual risk and the reason it is written down.

### Verified

- **A nested CTOC tree round-trips through mutagen with its hierarchy intact.** A root TOC
  naming a child TOC and a chapter, plus the child TOC naming two chapters, reads back from
  mutagen with the same membership, the same child order, and the top-level and ordered
  flags on the right frames \u{2014} which is the check that would have caught 0.2.0's
  transposed flags from a real file rather than from a hand-written fixture.

  Also verified there: a chapter with no end serializes `0xFFFFFFFF`, the spec's
  ignore-sentinel, and mutagen reports it as `end=4294967295` rather than as an error \u{2014}
  so the sentinel is the interoperable way to say "open-ended", and not something to avoid.

## [0.2.0] - 2026-10-06

MP3 is the dominant audiobook format, and `CHAP`/`CTOC` had never been checked against a
real file. Doing that turned up three defects, two of which made a whole class of tags
unreadable.

### Fixed

- **`CTOC` top-level and ordered flags were transposed.** Top-level is bit 1 and ordered
  is bit 0; this crate read bit 0 as top-level and bit 1 as ordered. A TOC that is top
  level and not ordered came back as the exact opposite, so a caller hunting for the root
  found none and the chapters lost their order \u{2014} while every existing test passed,
  because the only CTOCs in the corpus were ones this crate wrote.

  Established from two implementations sharing no code: mutagen's
  `CTOCFlags.TOP_LEVEL = 0x2`, `ORDERED = 0x1`, and TagLib's
  `d->isTopLevel = data & 2; d->isOrdered = data & 1`. A file mutagen wrote with
  `flags=CTOCFlags.TOP_LEVEL` carries `0x02` on the wire, and that byte is the fixture.

  The specification's `%000000ab` diagram reads as "six zero bits, then a, then b" \u{2014} a
  is bit 1 counting from the low end. Reading it as a = bit 7 is what produced the bug.

- **An ID3v2.3 tag could not be parsed at all.** Every frame was read with v2.4's
  ten-byte header instead of v2.3's nine, so each frame's trailing flag byte was taken as
  the first byte of the next frame's id. `Tag::parse` returned an error, which means
  `MediaProbe` silently lost the title, the artist and every other field of every v2.3
  file \u{2014} not just the chapters.

  **The nine-byte header is what the specification says, and nothing uses it.** mutagen
  emits ten for a v2.3 tag and ffmpeg reads ten (`case 3: case 4: taghdrlen = 10;` in
  `libavformat/id3v2.c`, distinguishing the versions only in how the size is decoded). So
  a conformant nine-byte tag is a file no major tool can read, while a ten-byte one is
  what a real v2.3 file contains. Picking either alone breaks half the world, so both are
  now accepted: the ten-byte shape is tried first and the frame ids are checked for being
  four alphanumerics, falling back to nine when that fails. The check is unambiguous in
  both directions \u{2014} a nine-byte file read as ten makes the size field absorb the
  flag byte and overrun the tag, and a ten-byte file read as nine starts the next frame on
  the second flag byte, which is never alphanumeric.

  Writing still emits the ten-byte shape, because writing a file no major tool can read is
  not a defence of the specification.

### Added

- **`Chapter::with_title`, `set_title`, `title`, `subframe` and `remove_subframe`** \u{2014}
  reading a chapter title was public but writing one was not. The sub-frame list is a pair
  of an id and a raw body, so a caller emitting chapters had to hand-encode a text frame
  to get a title into a file, which left the encoding choice and the trailing NUL to
  whoever held the bytes. That is how the two ends of this crate drifted apart.
  `resolve` now uses the same accessor, so a title is read identically either way.
- **`Chapter::remove_subframe`**, and `set_title` replaces rather than accumulates: two
  `TIT2` sub-frames read back as whichever a reader finds first, so an accumulating setter
  makes a title depend on the reader.

### Established about the format, by running the tools rather than reading about them

- **ID3v2.3 does not define UTF-8.** Encoding byte 3 arrived in v2.4. A v2.3 frame tagged
  UTF-8 is invalid and both mutagen and ffprobe discard the whole frame on sight \u{2014} a
  tag that looks perfectly well-formed and reads as no metadata at all. v2.3 text must be
  Latin-1 or UTF-16 with a BOM.
- **v2.3 `CHAP` and `CTOC` are interoperable**, provided the text encoding is valid: once
  a correctly framed v2.3 file is written, mutagen, ffprobe and this crate all read the
  same chapters and titles from it.
- **mutagen's own v2.3 chapter output is malformed** \u{2014} it writes a spurious leading
  byte in the element id and an off-by-one frame size, and compensates when reading its
  own output back.
## [0.1.0] - 2026-10-05

First release. `id3-core` is the estate's L0 audio-metadata layer: the tag
formats every audio file carries, read and written without depending on
`std`.

### Added

- **ID3v2.2 / v2.3 / v2.4** — header and frame table, the three- to
  four-character frame id upgrade table, per-version frame-size encoding (plain
  big-endian in v2.3, synch-safe in v2.4), extended headers (both size
  conventions), frame padding, and unsynchronisation in both directions.
- **ID3v1 / ID3v1.1** — `TagV1::find` locates the 128-byte trailer at a fixed
  offset, and the v1.1 track-number marker in the comment field is detected
  explicitly.
- **Text frames** in all four encodings — Latin-1, BOM-marked UTF-16, UTF-16BE,
  UTF-8 — with surrogate-pair validation, BOM handling, and Latin-1 decoded
  rather than rejected.
- **`CHAP` / `CTOC`** — chapters and tables of contents, including sub-frames,
  the spec's `0xFFFFFFFF` "unset end time", and `chapter::resolve` returning
  playing order sorted by start time with TOC depth attached.
- **`APIC` / `PIC`** — cover art, with MIME sniffing on the way in and the v2.2
  three-byte image format mapped onto the equivalent MIME type.
- **`COMM` / `USLT`** — comments and lyrics with ISO-639-2 language codes.
- **`synchsafe`** — strict and lenient decoders; the strict one rejects a
  non-synch-safe writer, the lenient one recovers.
- **Unknown frames preserved verbatim** as `Frame::Unknown`, so a tag written by
  a newer encoder survives a read/write cycle.

### Verified against

- EBU-style reference signals are not applicable here, but the format claims are
  pinned to the specification: synch-safe encoding against the ID3v2.4 §6.2
  worked examples (`255` → `00 00 01 7F`), the v2.2 id upgrade table against the
  v2.3 renaming, and a hand-built extended-header tag.

### Known limitations

- **No image decoding.** `APIC` yields raw bytes and a sniffed MIME type.
  Validating or transcoding image payloads belongs to `media-kit`.
- **No container parsing.** This crate reads tag formats, not file formats;
  `WAV`/`AIFF` chunk wrappers and MP4 `moov/udta` atoms are separate concerns.
- **No Vorbis comments.** Ogg-container keys are a different specification with a
  different escaping rule.
- **A zero-length frame is not representable.** ID3v2 reserves a frame size of
  zero as the frame-table padding marker, so it terminates the table. An empty
  *text* value is still expressible (it carries a one-byte encoding selector);
  an empty *unknown* frame is not.
- **A NUL-padded v1.0 tag is indistinguishable from v1.1**, because a zero in
  the second-to-last comment byte *is* the v1.1 marker. Documented rather than
  worked around, since "fixing" it would break genuine v1.1 tags.