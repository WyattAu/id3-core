# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

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