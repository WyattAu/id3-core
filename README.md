# id3-core

ID3v1 / ID3v2 audio tag reader and writer for Rust: v2.2/2.3/2.4 frames,
`CHAP`/`CTOC` chapters, `APIC` cover art, unsynchronisation, `no_std`.

**Layer:** L0: leaf · **Estate deps:** none · **Runtime deps:** `thiserror`

Part of the WyattAu estate's audio path: `id3-core` reads the tags, `dsp-core`
measures the audio, and an audiobook kit composes both. See
[WyattAu/engineering-standards](https://github.com/WyattAu/engineering-standards).

## Why

Metadata is the difference between a folder of files and a library. A 40-hour
audiobook with `CHAP`/`CTOC` is navigable; without them it is a seek bar. No
estate crate read audio tags, so this is the layer everything downstream of
"what file is this" sits on.

It is also the layer where the format's sharp edges are: unsynchronisation,
three text encodings that are genuinely different wire formats, a frame-size
field that changed encoding between spec revisions, and a v2.2-to-v2.3 frame-id
rename. Those are handled here rather than in every consumer.

## Install

```toml
[dependencies]
id3-core = "0.1"
```

## What it covers

| Area | Notes |
|---|---|
| ID3v2.2 / v2.3 / v2.4 | header, frame table, extended headers, frame padding |
| Frame sizes | plain big-endian (v2.3) vs synch-safe (v2.4), decided by version |
| Frame ids | full v2.2 three-character upgrade table |
| Unsynchronisation | `synchronise` / `unsynchronise`, lossless both ways |
| Text encodings | Latin-1, UTF-16+BOM, UTF-16BE, UTF-8, with surrogate validation |
| `CHAP` / `CTOC` | chapters, TOCs, sub-frames, and `resolve` into playing order |
| `APIC` / `PIC` | cover art, MIME sniffing, v2.2 format mapping |
| `COMM` / `USLT` | comments and lyrics with ISO-639-2 language codes |
| ID3v1 / v1.1 | 128-byte trailer, track-number marker |
| Unknown frames | preserved verbatim rather than dropped |

## What it deliberately does not do

- **No image decoding.** `APIC` yields raw bytes and a sniffed MIME type;
  validating image payloads belongs to `media-kit`.
- **No container parsing.** This crate reads tag formats, not file formats. The
  `WAV`/`AIFF` chunk wrappers and MP4 `moov`/`udta` atoms are separate
  concerns.
- **No Vorbis comments.** Ogg-container keys are a different specification with
  different escaping rules.

## Where the wild disagrees with the specification

Two places, both settled by running the tools rather than by reading the specification,
and both of which changed this crate's behaviour:

- **ID3v2.3's frame header is nine bytes in the specification and ten in practice.**
  mutagen emits ten and ffmpeg reads ten (`case 3: case 4: taghdrlen = 10;`). A conformant
  nine-byte tag is a file no major tool can read, so this crate tries ten first and falls
  back to nine when the frame ids do not come out as four alphanumerics. It *writes* ten,
  because writing an unreadable file defends nothing. Before this, every ID3v2.3 tag
  failed to parse entirely, taking the title, artist and chapters with it.
- **`CTOC`'s top-level flag is bit 1 and ordered is bit 0.** mutagen's
  `CTOCFlags.TOP_LEVEL = 0x2` and TagLib's `d->isTopLevel = data & 2` agree. The
  specification's `%000000ab` diagram reads as "six zero bits, then a, then b", which is
  a at bit 1 counting from the low end. Reading it as a = bit 7 is what transposed them.

Also worth knowing: **ID3v2.3 has no UTF-8.** Encoding byte 3 arrived in v2.4, and a v2.3
frame tagged UTF-8 is discarded whole by mutagen and ffprobe, leaving a well-formed tag
that reads as no metadata at all.

## Example

Read a tag:

```rust
use id3_core::{Tag, TextField};

let bytes = std::fs::read("book.mp3")?;
let tag = Tag::parse(&bytes)?;

println!("{:?}", tag.text_field(TextField::Title));
```

Find a tag in a file that may not start with one:

```rust
use id3_core::find_id3v2;

let data = std::fs::read("book.mp3")?;
if let Some(len) = find_id3v2(&data) {
    let tag = id3_core::Tag::parse(&data[..len])?;
    // ...
}
```

Build an audiobook tag with chapters:

```rust
use id3_core::{Chapter, Frame, Tag, TextField, TextEncoding, Toc};

let mut tag = Tag::new();
tag.set_text(TextField::Title, TextEncoding::Utf8, "The Audiobook");

let mut toc = Toc::root("book");
for (id, start, end) in [("ch1", 0u32, 900_000), ("ch2", 900_000, 1_800_000)] {
    tag.push(Frame::Chapter(Chapter::new(id, start).with_end(end)));
    toc.push(id);
}
tag.push(Frame::Toc(toc));

let chapters = tag.resolved_chapters()?;   // playing order, TOC depth attached
```

## Guarantees

- **No panic on malformed input.** Every parse path returns `Result`; the
  `deny(panic)` lint is on for the whole crate.
- **No indexing in library code** (`deny(indexing_slicing)`), so every slice
  access is bounds-checked at the source rather than trusted.
- **No allocation in the frame-id or size paths**: the parse allocates only for
  the frames it actually returns.
- **`no_std` + `alloc`**, verified on `thumbv7em-none-eabihf`. The embedded
  path is what makes this usable inside a player without a runtime.
- **Unknown frames survive a round trip.** A tag written by a newer encoder is
  not silently truncated by reading and rewriting it here.
- **Version-correct sizes.** A v2.4 frame over 128 bytes is mis-sized by a
  heuristic that guesses between plain and synch-safe; the version decides, and
  a test pins it.

## Gates

- `cargo test`: 106 unit tests, 2 integration, 2 doc tests
- proptest on synch-safe round-trip, tag round-trip, and Latin-1 totality
- `cargo clippy --all-features --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo check --target thumbv7em-none-eabihf --no-default-features`
- estate `feature-compat` gate

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option.