//! Clef media boundary tests. Fixtures are generated locally with Pillow, not copied.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use jev_core::{
    Content, EmbeddedImage, EvaluationRequest, MediaError, ModelId, Question, QuestionId, State,
};
use serde_json::json;

const PNG: &[u8] = include_bytes!("fixtures/two-by-three.png");
const JPEG: &[u8] = include_bytes!("fixtures/two-by-three.jpg");
const WEBP: &[u8] = include_bytes!("fixtures/two-by-three.webp");

fn request() -> Result<EvaluationRequest, Box<dyn std::error::Error>> {
    Ok(EvaluationRequest::new(
        State::text("What is visible?")?,
        ModelId::new("clef")?,
        vec![(
            QuestionId::new("visible")?,
            Question::noul(Content::text("Is it blue?")?, None)?,
        )],
    )?)
}

#[test]
fn supported_image_headers_determine_mime_and_dimensions() {
    for (bytes, mime) in [
        (PNG, "image/png"),
        (JPEG, "image/jpeg"),
        (WEBP, "image/webp"),
    ] {
        let image = EmbeddedImage::from_bytes(bytes.to_vec()).unwrap();
        assert_eq!(image.content_type(), mime);
        assert_eq!((image.width(), image.height()), (2, 3));
        assert_eq!(image.bytes(), bytes);
    }
}

#[test]
fn cloning_images_for_batch_templates_shares_immutable_bytes() {
    let image = EmbeddedImage::from_bytes(PNG.to_vec()).unwrap();
    let copies = vec![image.clone(); 100];
    assert!(
        copies
            .iter()
            .all(|copy| std::ptr::eq(image.bytes(), copy.bytes()))
    );
    assert_eq!(copies.first(), Some(&image));
}

#[test]
fn request_serializes_images_and_capacity_option_only_when_supplied() {
    let plain = request().unwrap();
    let value = serde_json::to_value(&plain).unwrap();
    assert!(value.get("images").is_none());
    assert!(value.get("options").is_none());
    let image = EmbeddedImage::from_bytes(PNG.to_vec()).unwrap();
    let encoded_image = serde_json::to_value(&image).unwrap();
    let enriched = plain
        .with_images(vec![image])
        .unwrap()
        .with_reject_if_busy(true);
    let value = serde_json::to_value(&enriched).unwrap();
    assert_eq!(value["images"], json!([encoded_image]));
    assert_eq!(value["options"], json!({"rejectIfBusy":true}));
    assert!(
        serde_json::to_value(enriched.with_reject_if_busy(false))
            .unwrap()
            .get("options")
            .is_none()
    );
}

#[test]
fn request_rejects_five_images() {
    let image = EmbeddedImage::from_bytes(PNG.to_vec()).unwrap();
    assert!(matches!(
        request().unwrap().with_images(vec![image; 5]),
        Err(MediaError::TooManyImages)
    ));
}

#[test]
fn embedded_json_accepts_both_forms_without_disclosing_content_in_debug() {
    for (bytes, mime) in [
        (PNG, "image/png"),
        (JPEG, "image/jpeg"),
        (WEBP, "image/webp"),
    ] {
        let encoded = STANDARD.encode(bytes);
        let object = json!({"content_type":mime,"base64":encoded});
        let image: EmbeddedImage = serde_json::from_value(object.clone()).unwrap();
        let from_url: EmbeddedImage =
            serde_json::from_value(json!(format!("data:{mime};base64,{encoded}"))).unwrap();
        assert_eq!(image, from_url);
        assert_eq!(serde_json::to_value(&image).unwrap(), object);
        assert!(!format!("{image:?}").contains(&encoded));
        assert!(!format!("{image:?}").contains(&format!("{bytes:?}")));
    }
}

#[test]
fn image_objects_reject_duplicate_unknown_and_missing_fields() {
    let encoded = STANDARD.encode(PNG);
    for json in [
        format!(r#"{{"content_type":"image/png","base64":"{encoded}","base64":"{encoded}"}}"#),
        format!(
            r#"{{"content_type":"image/png","content_type":"image/png","base64":"{encoded}"}}"#
        ),
        format!(
            r#"{{"content_type":"image/png","base64":"{encoded}","url":"https://example.test/image"}}"#
        ),
        r#"{"content_type":"image/png"}"#.to_owned(),
        format!(r#"{{"base64":"{encoded}"}}"#),
    ] {
        assert!(serde_json::from_str::<EmbeddedImage>(&json).is_err());
    }
}

#[test]
fn image_encoding_rejects_urls_wrong_mime_and_invalid_base64() {
    let encoded = STANDARD.encode(PNG);
    assert_eq!(
        EmbeddedImage::from_base64("image/jpeg", &encoded),
        Err(MediaError::ContentTypeMismatch)
    );
    for mime in ["image/gif", "image/svg+xml", "text/plain"] {
        assert_eq!(
            EmbeddedImage::from_base64(mime, &encoded),
            Err(MediaError::UnsupportedFormat)
        );
    }
    for encoded in ["not base64!", "AA=A", "A===", "Zh=="] {
        assert_eq!(
            EmbeddedImage::from_base64("image/png", encoded),
            Err(MediaError::InvalidBase64)
        );
    }
    for value in [
        "https://example.test/p.png",
        "file:///tmp/p.png",
        "data:image/png,x",
        "data:image/png;utf8,x",
    ] {
        assert!(EmbeddedImage::from_data_url(value).is_err());
    }
}

#[test]
fn data_url_tokens_are_ascii_case_insensitive_without_broadening_typed_mime() {
    for (bytes, mime) in [
        (PNG, "ImAgE/PnG"),
        (JPEG, "IMAGE/JPEG"),
        (WEBP, "Image/WebP"),
    ] {
        let encoded = STANDARD.encode(bytes);
        let image = EmbeddedImage::from_data_url(&format!("DaTa:{mime};BASE64,{encoded}")).unwrap();
        assert_eq!(image.bytes(), bytes);
        assert!(EmbeddedImage::from_base64(mime, &encoded).is_err());
        assert!(
            EmbeddedImage::from_data_url(&format!("data:{mime};charset=utf8;BASE64,{encoded}"))
                .is_err()
        );
    }
}

#[test]
fn native_raw_base64_images_are_sniffed_without_accepting_raw_strings_in_cloudflare_json() {
    for (bytes, mime) in [
        (PNG, "image/png"),
        (JPEG, "image/jpeg"),
        (WEBP, "image/webp"),
    ] {
        let encoded = STANDARD.encode(bytes);
        assert_eq!(
            EmbeddedImage::from_raw_base64(&encoded)
                .unwrap()
                .content_type(),
            mime
        );
        assert!(serde_json::from_value::<EmbeddedImage>(json!(encoded)).is_err());
    }
    assert!(EmbeddedImage::from_raw_base64("https://example.test/i").is_err());
    assert!(EmbeddedImage::from_raw_base64("A===").is_err());
}

#[test]
fn image_size_pixel_and_aggregate_limits_have_inclusive_boundaries() {
    let mut bytes = PNG.to_vec();
    bytes.resize(4 * 1024 * 1024, 0);
    let largest = EmbeddedImage::from_bytes(bytes.clone()).unwrap();
    bytes.push(0);
    assert_eq!(
        EmbeddedImage::from_bytes(bytes),
        Err(MediaError::ImageTooLarge)
    );
    assert!(
        request()
            .unwrap()
            .with_images(vec![largest.clone(), largest.clone()])
            .is_ok()
    );
    assert!(matches!(
        request().unwrap().with_images(vec![
            largest.clone(),
            largest,
            EmbeddedImage::from_bytes(PNG.to_vec()).unwrap()
        ]),
        Err(MediaError::TotalImagesTooLarge)
    ));
    for (width, height, accepted) in [
        (4000u32, 4000u32, true),
        (4001, 4000, false),
        (0, 1, false),
        (u32::MAX, u32::MAX, false),
    ] {
        let mut bytes = PNG.to_vec();
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        assert_eq!(EmbeddedImage::from_bytes(bytes).is_ok(), accepted);
    }
    let encoded = "A".repeat((4 * 1024 * 1024usize).div_ceil(3) * 4 + 1);
    assert_eq!(
        EmbeddedImage::from_base64("image/png", &encoded),
        Err(MediaError::ImageTooLarge)
    );
}

#[test]
fn corrupt_full_png_signature_is_not_accepted_as_png() {
    let mut bytes = PNG.to_vec();
    bytes[4] = 0;
    assert!(EmbeddedImage::from_bytes(bytes).is_err());
}

#[test]
fn truncated_headers_are_refused_for_every_supported_format() {
    for bytes in [PNG, JPEG, WEBP] {
        for len in 0..24.min(bytes.len()) {
            assert!(EmbeddedImage::from_bytes(bytes[..len].to_vec()).is_err());
        }
    }
}

proptest::proptest! {
    #[test]
    fn hostile_bytes_cannot_escape_media_bounds(bytes in proptest::collection::vec(proptest::num::u8::ANY,0..4096)) {
        if let Ok(image)=EmbeddedImage::from_bytes(bytes) {
            proptest::prop_assert!(matches!(image.content_type(),"image/png"|"image/jpeg"|"image/webp"));
            proptest::prop_assert!(image.width()>0 && image.height()>0);
            proptest::prop_assert!(image.width().checked_mul(image.height()).is_some_and(|pixels| pixels<=16_000_000));
        }
    }
}

#[test]
fn local_keep_alive_accepts_seconds_and_duration_strings_but_rejects_other_shapes() {
    for value in [json!("5m"), json!(0), json!(-1), json!(90)] {
        let request = request().unwrap().with_keep_alive(value.clone()).unwrap();
        assert_eq!(request.keep_alive(), Some(&value));
        assert_eq!(serde_json::to_value(request).unwrap()["keep_alive"], value);
    }
    for value in [
        json!(null),
        json!(true),
        json!(1.2),
        json!({}),
        json!([]),
        json!(""),
        json!("\u{1b}"),
    ] {
        assert!(request().unwrap().with_keep_alive(value).is_err());
    }
    assert!(
        serde_json::to_value(request().unwrap())
            .unwrap()
            .get("keep_alive")
            .is_none()
    );
}

#[test]
fn local_score_constructor_supports_26_without_weakening_standard_limit() {
    let levels = vec![Content::text("level").unwrap(); 26];
    assert!(Question::score(Content::text("? ").unwrap(), levels.clone()).is_err());
    assert!(Question::score_with_max(Content::text("?").unwrap(), levels, 26).is_ok());
    assert!(
        Question::score_with_max(
            Content::text("?").unwrap(),
            vec![Content::text("level").unwrap(); 27],
            26
        )
        .is_err()
    );
}
