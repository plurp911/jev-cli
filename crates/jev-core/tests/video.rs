//! Explicit frame sequences for the local Python bridge, with client resource bounds.

use jev_core::{
    Content, EmbeddedImage, EmbeddedVideo, EvaluationRequest, ModelId, Question, QuestionId, State,
};
use serde_json::json;

const PNG: &[u8] = include_bytes!("fixtures/two-by-three.png");

fn request() -> Result<EvaluationRequest, Box<dyn std::error::Error>> {
    Ok(EvaluationRequest::new(
        State::text("Inspect the clip")?,
        ModelId::new("clef")?,
        vec![(
            QuestionId::new("visible")?,
            Question::noul(Content::text("Is it blue?")?, None)?,
        )],
    )?)
}

fn frame() -> Result<EmbeddedImage, jev_core::MediaError> {
    EmbeddedImage::from_bytes(PNG.to_vec())
}

#[test]
fn video_preserves_frame_order_and_serializes_an_explicit_frame_object() {
    let first = frame().unwrap();
    let second =
        EmbeddedImage::from_bytes(include_bytes!("fixtures/two-by-three.jpg").to_vec()).unwrap();
    let video = EmbeddedVideo::new(vec![first.clone(), second.clone()]).unwrap();
    assert_eq!(video.frames(), &[first.clone(), second.clone()]);
    assert_eq!(video.byte_len(), first.byte_len() + second.byte_len());
    assert_eq!(
        serde_json::to_value(&video).unwrap(),
        json!({"frames":[first,second]})
    );
    let serialized = serde_json::to_string(&video).unwrap();
    assert_eq!(
        serde_json::from_str::<EmbeddedVideo>(&serialized).unwrap(),
        video
    );
    assert!(!format!("{video:?}").contains(&frame().unwrap().base64()));
}

#[test]
fn video_frame_count_and_dimensions_are_validated() {
    assert!(EmbeddedVideo::new(Vec::new()).is_err());
    assert!(EmbeddedVideo::new(vec![frame().unwrap(); 32]).is_ok());
    assert!(EmbeddedVideo::new(vec![frame().unwrap(); 33]).is_err());
    let mut mismatched = PNG.to_vec();
    mismatched[16..20].copy_from_slice(&3u32.to_be_bytes());
    assert!(
        EmbeddedVideo::new(vec![
            frame().unwrap(),
            EmbeddedImage::from_bytes(mismatched).unwrap()
        ])
        .is_err()
    );
}

#[test]
fn videos_reject_unknown_duplicate_and_path_fields() {
    let encoded = serde_json::to_string(&frame().unwrap()).unwrap();
    for text in [
        format!("{{\"frames\":[{encoded}],\"frames\":[{encoded}]}}"),
        format!("{{\"frames\":[{encoded}],\"url\":\"https://example.test/clip.mp4\"}}"),
        "{\"frames\":[]}".to_owned(),
        "{\"path\":\"/tmp/clip.mp4\"}".to_owned(),
        "{\"frames\":[\"file:///tmp/frame.png\"]}".to_owned(),
    ] {
        assert!(serde_json::from_str::<EmbeddedVideo>(&text).is_err());
    }
}

#[test]
fn combined_media_budget_is_enforced_in_both_setter_orders() {
    let mut bytes = PNG.to_vec();
    bytes.resize(4 * 1024 * 1024, 0);
    let large = EmbeddedImage::from_bytes(bytes).unwrap();
    let video = EmbeddedVideo::new(vec![large.clone()]).unwrap();
    assert!(
        request()
            .unwrap()
            .with_videos(vec![video.clone()])
            .unwrap()
            .with_images(vec![large.clone()])
            .is_ok()
    );
    assert!(
        request()
            .unwrap()
            .with_images(vec![large.clone()])
            .unwrap()
            .with_videos(vec![video.clone()])
            .is_ok()
    );
    assert!(
        request()
            .unwrap()
            .with_videos(vec![video.clone()])
            .unwrap()
            .with_images(vec![large.clone(), frame().unwrap()])
            .is_err()
    );
    assert!(
        request()
            .unwrap()
            .with_images(vec![large.clone(), frame().unwrap()])
            .unwrap()
            .with_videos(vec![video.clone()])
            .is_err()
    );
    assert!(
        request()
            .unwrap()
            .with_videos(vec![
                EmbeddedVideo::new(vec![large.clone(), large, frame().unwrap()]).unwrap()
            ])
            .is_err()
    );
    assert!(
        request()
            .unwrap()
            .with_videos(vec![EmbeddedVideo::new(vec![frame().unwrap()]).unwrap(); 5])
            .is_err()
    );
}

#[test]
fn request_omits_empty_video_and_local_controls_and_preserves_explicit_values() {
    let plain = serde_json::to_value(request().unwrap()).unwrap();
    for key in ["videos", "max_length", "media_kwargs"] {
        assert!(plain.get(key).is_none());
    }
    let kwargs = json!({"max_pixels":1_048_576});
    let request = request()
        .unwrap()
        .with_videos(vec![EmbeddedVideo::new(vec![frame().unwrap()]).unwrap()])
        .unwrap()
        .with_max_length(16384)
        .unwrap()
        .with_media_kwargs(kwargs.clone())
        .unwrap();
    assert_eq!(request.max_length(), Some(16384));
    assert_eq!(request.media_kwargs(), Some(&kwargs));
    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(encoded["videos"].as_array().unwrap().len(), 1);
    assert_eq!(encoded["max_length"], json!(16384));
    assert_eq!(encoded["media_kwargs"], kwargs);
}

#[test]
fn local_max_length_has_inclusive_safe_bounds() {
    for value in [1, 16384, 65536] {
        assert!(request().unwrap().with_max_length(value).is_ok());
    }
    for value in [0, 65537, u32::MAX] {
        assert!(request().unwrap().with_max_length(value).is_err());
    }
}

#[test]
fn aggregate_pixels_are_bounded_before_any_video_decompression() {
    let mut bytes = PNG.to_vec();
    bytes[16..20].copy_from_slice(&4000u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&4000u32.to_be_bytes());
    let large = EmbeddedImage::from_bytes(bytes).unwrap();
    assert!(EmbeddedVideo::new(vec![large.clone(); 4]).is_ok());
    assert!(EmbeddedVideo::new(vec![large.clone(); 5]).is_err());
    let video = EmbeddedVideo::new(vec![large.clone(); 3]).unwrap();
    assert!(
        request()
            .unwrap()
            .with_images(vec![large.clone()])
            .unwrap()
            .with_videos(vec![video.clone()])
            .is_ok()
    );
    assert!(
        request()
            .unwrap()
            .with_images(vec![large.clone(); 2])
            .unwrap()
            .with_videos(vec![video.clone()])
            .is_err()
    );
    assert!(
        request()
            .unwrap()
            .with_videos(vec![video.clone()])
            .unwrap()
            .with_images(vec![large; 2])
            .is_err()
    );
    assert!(
        request()
            .unwrap()
            .with_videos(vec![video.clone(), video])
            .is_err()
    );
}

#[test]
fn processor_options_accept_only_bounded_documented_numeric_and_boolean_controls() {
    for value in [
        json!({}),
        json!({"min_pixels":1,"max_pixels":16_000_000}),
        json!({"fps":0.5,"do_sample_frames":true}),
        json!({"num_frames":32,"do_sample_frames":true}),
        json!({"fps":120}),
    ] {
        assert!(request().unwrap().with_media_kwargs(value).is_ok());
    }
    for value in [
        json!(null),
        json!([]),
        json!(true),
        json!({"text":"secret-payload"}),
        json!({"images":[]}),
        json!({"return_tensors":"pt"}),
        json!({"videos_kwargs":{"fps":1}}),
        json!({"max_pixels":0}),
        json!({"max_pixels":16_000_001}),
        json!({"min_pixels":-1}),
        json!({"max_pixels":1.5}),
        json!({"min_pixels":100,"max_pixels":99}),
        json!({"fps":0}),
        json!({"fps":-1}),
        json!({"fps":120.1}),
        json!({"fps":"24"}),
        json!({"num_frames":0}),
        json!({"num_frames":33}),
        json!({"num_frames":1.2}),
        json!({"do_sample_frames":1}),
        json!({"do_sample_frames":null}),
    ] {
        let error = request().unwrap().with_media_kwargs(value).unwrap_err();
        assert!(!error.to_string().contains("secret-payload"));
    }
}

#[test]
fn video_sampling_fps_and_num_frames_are_mutually_exclusive() {
    assert!(
        request()
            .unwrap()
            .with_media_kwargs(json!({"fps":1,"num_frames":2,"do_sample_frames":true}))
            .is_err()
    );
}

proptest::proptest! {
    #[test]
    fn validated_video_dimensions_and_count_survive_json(count in 1usize..33) {
        let video=EmbeddedVideo::new(vec![frame().unwrap();count]).unwrap();
        let encoded=serde_json::to_string(&video).unwrap();
        let decoded: EmbeddedVideo=serde_json::from_str(&encoded).unwrap();
        proptest::prop_assert_eq!(decoded.frames().len(),count);
        proptest::prop_assert!(decoded.frames().iter().all(|image| image.width()==2 && image.height()==3));
    }
}

#[test]
fn video_metadata_preserves_source_cadence_and_sparse_indices() {
    let value = json!({"frames":[frame().unwrap(),frame().unwrap()],
        "metadata":{"fps":30,"total_num_frames":90,"frames_indices":[0,60],"duration":3}});
    let video: EmbeddedVideo = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(video).unwrap(), value);
}

#[test]
fn video_metadata_rejects_unknown_and_misaligned_fields() {
    for metadata in [
        json!({"fps":0}),
        json!({"fps":true}),
        json!({"fps":121}),
        json!({"fps":2,"url":"x"}),
        json!({"fps":2,"frames_indices":[0]}),
        json!({"fps":2,"frames_indices":[1,0]}),
        json!({"fps":2,"total_num_frames":1}),
        json!({"fps":2,"duration":0}),
        json!({"fps":2,"total_num_frames":8,"frames_indices":[0,8]}),
    ] {
        assert!(
            serde_json::from_value::<EmbeddedVideo>(
                json!({"frames":[frame().unwrap(),frame().unwrap()],"metadata":metadata})
            )
            .is_err()
        );
    }
}

#[test]
fn video_metadata_rejects_duplicate_fields_and_unbounded_source_time() {
    let encoded = serde_json::to_string(&frame().unwrap()).unwrap();
    let raw = format!(r#"{{"frames":[{encoded}],"metadata":{{"fps":30,"fps":24}}}}"#);
    assert!(serde_json::from_str::<EmbeddedVideo>(&raw).is_err());
    for metadata in [
        json!({"fps":1e-12}),
        json!({"fps":1,"total_num_frames":10_000_000}),
    ] {
        assert!(
            serde_json::from_value::<EmbeddedVideo>(
                json!({"frames":[frame().unwrap()],"metadata":metadata})
            )
            .is_err()
        );
    }
}

#[test]
fn independent_state_token_budget_serializes_zero_and_inclusive_maximum() {
    for value in [0, 1, 65536] {
        let encoded =
            serde_json::to_value(request().unwrap().with_max_state_tokens(value).unwrap()).unwrap();
        assert_eq!(encoded["max_state_tokens"], value);
        assert!(encoded.get("max_length").is_none());
    }
    assert!(request().unwrap().with_max_state_tokens(65537).is_err());
}
