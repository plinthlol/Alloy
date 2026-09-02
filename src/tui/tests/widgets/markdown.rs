// unit tests for the markdown document pipeline in tui/widgets/markdown.rs.
// kept here (via #[path]) rather than inline so the 1800-line widget stays
// focused on rendering; this module is a child of `markdown`, so private
// helpers are directly testable.

use super::*;

// --- strip_duplicate_title -------------------------------------------------

#[test]
fn strip_duplicate_title_removes_leading_h1_match() {
    let out = strip_duplicate_title("# Sodium\n\nReal body here.\n", "Sodium");
    assert_eq!(out, "Real body here.\n");
}

#[test]
fn strip_duplicate_title_is_case_insensitive() {
    let out = strip_duplicate_title("# SODIUM\n\nbody", "sodium");
    assert_eq!(out, "body");
}

#[test]
fn strip_duplicate_title_keeps_non_matching_heading() {
    let source = "# Something Else\n\nbody";
    assert_eq!(strip_duplicate_title(source, "Sodium"), source);
}

// --- split_images ----------------------------------------------------------

fn image_urls_of(blocks: &[DocumentBlock]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|block| match block {
            DocumentBlock::ImageRow { images, .. } => {
                images.iter().map(|i| i.url.clone()).collect::<Vec<_>>()
            }
            DocumentBlock::Text(_) => Vec::new(),
        })
        .collect()
}

#[test]
fn split_images_extracts_image_urls() {
    let blocks = split_images(
        "intro\n\n![alt](https://example.com/a.png)\n\noutro",
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(image_urls_of(&blocks), vec!["https://example.com/a.png"]);
}

#[test]
fn split_images_groups_adjacent_images_into_one_row() {
    let blocks = split_images(
        "![](https://a/1.png) ![](https://a/2.png)",
        &Default::default(),
        &Default::default(),
    );
    let rows = blocks
        .iter()
        .filter(|b| matches!(b, DocumentBlock::ImageRow { .. }))
        .count();
    assert_eq!(rows, 1, "adjacent images belong in a single row");
}

#[test]
fn split_images_keeps_linked_image_range_valid() {
    let blocks = split_images(
        "[![alt](https://a/1.png)](https://example.com)",
        &Default::default(),
        &Default::default(),
    );
    let urls = image_urls_of(&blocks);
    assert_eq!(urls, vec!["https://a/1.png"]);
    match &blocks[0] {
        DocumentBlock::ImageRow { images, .. } => {
            assert_eq!(images[0].link.as_deref(), Some("https://example.com"));
        }
        DocumentBlock::Text(_) => panic!("expected an image row"),
    }
}

// --- pixel_size ------------------------------------------------------------

#[test]
fn pixel_size_parses_digits_and_px_suffix() {
    assert_eq!(pixel_size("640"), Some(640));
    assert_eq!(pixel_size("640px"), Some(640));
    assert_eq!(pixel_size(" 100% "), None);
    assert_eq!(pixel_size(""), None);
}

// --- text helpers ----------------------------------------------------------

#[test]
fn split_display_width_never_exceeds_width() {
    for line in split_display_width("hello world, this is long", 7) {
        assert!(Span::raw(&line).width() <= 7);
    }
}

#[test]
fn is_list_marker_matches_expected_shapes() {
    assert!(is_list_marker("-"));
    assert!(is_list_marker("•"));
    assert!(is_list_marker("1."));
    assert!(!is_list_marker("- item"));
    assert!(!is_list_marker("abc."));
}

#[test]
fn wrap_styled_spans_wraps_overlong_single_span() {
    let spans = vec![Span::raw("aaaa bbbb cccc")];
    let lines = wrap_styled_spans(&spans, 9);
    assert!(lines.len() >= 2);
    assert!(lines
        .iter()
        .all(|line| line.iter().map(Span::width).sum::<usize>() <= 9));
}

// --- decode_image ----------------------------------------------------------

#[test]
fn decode_image_parses_a_png() {
    let png = image::DynamicImage::new_rgb8(4, 4);
    let mut bytes = std::io::Cursor::new(Vec::new());
    png.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let decoded = decode_image(bytes.get_ref()).expect("png decodes");
    assert_eq!((decoded.width(), decoded.height()), (4, 4));
}

#[test]
fn decode_image_rejects_garbage() {
    assert!(decode_image(b"not an image at all").is_err());
}
