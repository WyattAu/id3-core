#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Cross-frame integration: the shapes a real tag actually has.
//!
//! The unit tests inside each module prove each frame round-trips alone. These
//! prove the combinations hold — an audiobook tag, a v2.2 legacy tag, a tag with
//! art and a comment, and a tag carrying bytes this crate does not understand.

use id3_core::{Chapter, Comment, Frame, Language, Tag, TextEncoding, TextField, Toc};

fn full_tag() -> Tag {
    let mut tag = Tag::new();
    tag.set_text(TextField::Title, TextEncoding::Utf8, "The Long Now");
    tag.set_text(TextField::Artist, TextEncoding::Utf8, "A. Writer");
    tag.set_text(TextField::Album, TextEncoding::Utf8, "Collected");
    tag.set_text(TextField::Year, TextEncoding::Utf8, "2026");
    tag.set_text(TextField::Track, TextEncoding::Utf8, "3/12");
    tag.set_text(TextField::Genre, TextEncoding::Utf8, "Science Fiction");

    tag.push(Frame::Comment(Comment::new(
        Language::new(*b"eng").expect("valid"),
        "narrated by the author",
        TextEncoding::Utf8,
    )));
    tag
}

#[test]
fn a_fully_populated_tag_round_trips() {
    let tag = full_tag();
    let bytes = tag.to_bytes().expect("serializes");
    let back = Tag::parse(&bytes).expect("reparses");

    assert_eq!(back.text_field(TextField::Title), Some("The Long Now"));
    assert_eq!(back.text_field(TextField::Artist), Some("A. Writer"));
    assert_eq!(back.text_field(TextField::Album), Some("Collected"));
    assert_eq!(back.text_field(TextField::Year), Some("2026"));
    assert_eq!(back.text_field(TextField::Track), Some("3/12"));
    assert_eq!(back.text_field(TextField::Genre), Some("Science Fiction"));
    assert_eq!(
        back.comment().map(|c| c.text.text()),
        Some("narrated by the author")
    );
}

#[test]
fn rewriting_a_field_leaves_no_duplicate() {
    let mut tag = full_tag();
    let before = tag.frames.len();
    tag.set_text(TextField::Title, TextEncoding::Utf8, "Renamed");

    let back = Tag::parse(&tag.to_bytes().expect("serializes")).expect("reparses");
    assert_eq!(back.text_field(TextField::Title), Some("Renamed"));
    let titles = back
        .frames
        .iter()
        .filter(|f| f.id().as_bytes() == b"TIT2")
        .count();
    assert_eq!(titles, 1, "a rewritten field must leave exactly one frame");
    assert!(before > 0);
}

#[test]
fn an_audiobook_tag_round_trips_with_its_chapters() {
    let mut tag = full_tag();
    let chapters: Vec<(&str, u32, u32)> = vec![
        ("front", 0, 45_000),
        ("ch1", 45_000, 945_000),
        ("ch2", 945_000, 1_845_000),
        ("ch3", 1_845_000, 2_700_000),
    ];
    let mut toc = Toc::root("book");
    for (id, start, end) in &chapters {
        tag.push(Frame::Chapter(Chapter::new(id, *start).with_end(*end)));
        toc.push(id);
    }
    tag.push(Frame::Toc(toc));

    let back = Tag::parse(&tag.to_bytes().expect("serializes")).expect("reparses");
    let resolved = back.resolved_chapters().expect("resolves");

    assert_eq!(resolved.len(), 4);
    assert_eq!(resolved[0].element_id, "front");
    assert_eq!(resolved[3].start_ms, 1_845_000);
    assert_eq!(resolved[3].duration_ms(), Some(855_000));

    // The invariant a player relies on: contiguous, non-overlapping chapters.
    for w in resolved.windows(2) {
        assert_eq!(
            w[0].end_ms,
            Some(w[1].start_ms),
            "chapters must abut: {:?} then {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn a_legacy_v22_tag_round_trips_its_three_character_ids() {
    let mut tag = Tag::with_version(2).expect("v2.2 is supported");
    tag.set_text(TextField::Title, TextEncoding::Latin1, "Cassette Era");
    tag.set_text(TextField::Artist, TextEncoding::Latin1, "Someone");

    let bytes = tag.to_bytes().expect("serializes");
    // The wire form really is three characters per frame id.
    assert!(
        bytes.windows(3).any(|w| w == b"TT2"),
        "a v2.2 tag must carry the three-character `TT2` id"
    );

    let back = Tag::parse(&bytes).expect("reparses");
    assert_eq!(back.major_version, 2);
    assert_eq!(back.text_field(TextField::Title), Some("Cassette Era"));
    assert_eq!(back.text_field(TextField::Artist), Some("Someone"));
}

#[test]
fn a_v23_tag_uses_plain_frame_sizes() {
    let mut tag = Tag::with_version(3).expect("v2.3 is supported");
    tag.set_text(TextField::Title, TextEncoding::Latin1, "Plain Sizes");

    let bytes = tag.to_bytes().expect("serializes");
    let back = Tag::parse(&bytes).expect("reparses");
    assert_eq!(back.major_version, 3);
    assert_eq!(back.text_field(TextField::Title), Some("Plain Sizes"));
}

#[test]
fn a_frame_over_128_bytes_is_sized_correctly() {
    // The case a "guess between plain and synch-safe" heuristic gets wrong: a
    // synch-safe 1024 reads as 2048 in plain form.
    let mut tag = Tag::new();
    tag.set_text(TextField::Title, TextEncoding::Utf8, "Big");
    tag.push(Frame::Unknown {
        id: id3_core::FrameId::raw(b"TXXX"),
        data: vec![0xA5; 1024],
    });

    let back = Tag::parse(&tag.to_bytes().expect("serializes")).expect("reparses");
    match back.frames.iter().find(|f| f.id().as_bytes() == b"TXXX") {
        Some(Frame::Unknown { data, .. }) => assert_eq!(data.len(), 1024),
        other => panic!("expected the unknown frame back, got {other:?}"),
    }
    assert_eq!(back.text_field(TextField::Title), Some("Big"));
}

#[test]
fn unsynchronised_art_survives_a_round_trip() {
    // Cover art is the largest frame in a real tag and the most likely to
    // contain 0xFF bytes that unsynchronisation must protect.
    let jpeg: Vec<u8> = std::iter::once(0xFFu8)
        .chain(std::iter::once(0xD8))
        .chain(std::iter::once(0xFF))
        .chain(std::iter::repeat_n(0xE0u8, 300))
        .collect();

    let mut tag = Tag::new();
    tag.unsynchronisation = true;
    tag.push(Frame::Picture(id3_core::AttachedPicture::cover(
        jpeg.clone(),
    )));

    let bytes = tag.to_bytes().expect("serializes");
    let back = Tag::parse(&bytes).expect("reparses");
    let cover = back.cover().expect("cover survives");
    assert_eq!(cover.mime_type, "image/jpeg");
    assert_eq!(cover.data, jpeg, "art bytes must be byte-identical");
}

#[test]
fn a_tag_with_a_utf16_description_and_non_ascii_title_survives() {
    let mut tag = Tag::new();
    tag.set_text(TextField::Title, TextEncoding::Utf16Bom, "Grüße aus Köln");
    tag.set_text(TextField::Artist, TextEncoding::Utf8, "Ünïcödé");

    let back = Tag::parse(&tag.to_bytes().expect("serializes")).expect("reparses");
    assert_eq!(back.text_field(TextField::Title), Some("Grüße aus Köln"));
    assert_eq!(back.text_field(TextField::Artist), Some("Ünïcödé"));
}

#[test]
fn an_empty_tag_and_a_tag_with_no_title_behave_the_same() {
    let empty = Tag::new();
    assert!(empty.text_field(TextField::Title).is_none());
    let back = Tag::parse(&empty.to_bytes().expect("serializes")).expect("reparses");
    assert!(back.text_field(TextField::Title).is_none());
    assert!(back.frames.is_empty());
}

#[test]
fn a_v1_tag_can_be_derived_from_a_v2_tag() {
    let mut tag = Tag::new();
    tag.set_text(TextField::Title, TextEncoding::Utf8, "Derive Me");
    tag.set_text(TextField::Artist, TextEncoding::Utf8, "Someone");
    tag.set_text(TextField::Year, TextEncoding::Utf8, "1999");
    tag.set_text(TextField::Track, TextEncoding::Utf8, "5/9");
    tag.set_text(TextField::Genre, TextEncoding::Utf8, "Rock");

    let v1 = id3_core::v1::from_v2(&tag);
    assert_eq!(v1.title, "Derive Me");
    assert_eq!(v1.track, Some(5));
    assert_eq!(v1.genre_name(), Some("Rock"));
    assert!(v1.fits(), "the derived tag must fit the fixed-width fields");

    let back = id3_core::TagV1::parse(&v1.to_bytes()).expect("reparses");
    assert_eq!(back.title, "Derive Me");
}
