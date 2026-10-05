//! The evaluation request: state, model, and a named set of questions.
//!
//! Shape from <https://docs.typesafe.ai/api>, "Request body".

use std::collections::BTreeSet;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::content::{Content, ContentError};
use crate::limits::MIN_QUESTIONS;
use crate::media::{
    EmbeddedImage, EmbeddedVideo, MAX_IMAGES, MAX_TOTAL_IMAGE_BYTES, MAX_TOTAL_MEDIA_PIXELS,
    MAX_VIDEOS, MediaError,
};
use crate::model::ModelId;
use crate::question::Question;

/// Reasons a question identifier is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QuestionIdError {
    /// The identifier was empty or whitespace only.
    #[error("a question id must not be empty")]
    Empty,
    /// The identifier contained a control character.
    ///
    /// Question ids are echoed in output and used as JSON keys, so a control character
    /// in one is both a rendering hazard and almost certainly a mistake. The API places
    /// no restriction on the key; this one is `jev`'s.
    #[error("a question id must not contain control characters")]
    ControlCharacter,
}

/// A key the caller chooses, under which the matching answer comes back.
///
/// The API states the key "is not sent to the underlying model and is not used in
/// inference", so it is purely a handle for code. Complete meaning belongs in the
/// question itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuestionId(String);

impl QuestionId {
    /// Validates and wraps an identifier.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionIdError`] for a blank or control-bearing identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, QuestionIdError> {
        let value = value.into();
        // Trimmed, like `ModelId`. Otherwise `" urgent"` and `"urgent"` are distinct
        // ids that duplicate detection cannot see, so a stray space in a request file
        // silently doubles a billed question.
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(QuestionIdError::Empty);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(QuestionIdError::ControlCharacter);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QuestionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The content every question in a request refers to.
///
/// The API accepts `string | object | array` here. A structured state with named
/// fields is the documented preference when the context has several parts; see
/// <https://docs.typesafe.ai/concepts/state>.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct State(Content);

impl State {
    /// Wraps validated content as request state.
    #[must_use]
    pub const fn new(content: Content) -> Self {
        Self(content)
    }

    /// Builds text state.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError::Empty`] for blank text. An empty state is rejected
    /// locally rather than sent: there is nothing for the model to evaluate.
    pub fn text(text: impl Into<String>) -> Result<Self, ContentError> {
        Content::text(text).map(Self)
    }

    /// Borrows the underlying content.
    #[must_use]
    pub const fn content(&self) -> &Content {
        &self.0
    }
}

/// Reasons a request cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RequestError {
    /// No questions were supplied.
    #[error("a request needs at least {MIN_QUESTIONS} question")]
    NoQuestions,
    /// Two questions shared an identifier.
    #[error("duplicate question id {id:?}")]
    DuplicateQuestionId {
        /// The repeated identifier.
        id: String,
    },
}

/// An invalid Ollama model retention setting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "keep_alive must be integer seconds or a nonempty duration string of at most 128 bytes without control characters"
)]
pub struct KeepAliveError;

/// An invalid local Python serving control. Errors never contain supplied values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LocalOptionsError {
    /// The requested context budget is outside the client limit.
    #[error("max_length must be an integer from 1 to 65536")]
    MaxLength,
    /// Textual state budget exceeds the local ceiling.
    #[error("max_state_tokens must be an integer from 0 to 65536")]
    MaxStateTokens,
    /// Processor controls are restricted to a bounded supported subset.
    #[error(
        "media_kwargs accepts only min_pixels/max_pixels (integers 1..16000000, min <= max), mutually exclusive fps (0 < fps <= 120) or num_frames (integer 1..32), and do_sample_frames (boolean)"
    )]
    MediaKwargs,
}

/// A complete System One evaluation request.
///
/// Question order is preserved so that human output reads in the order the user wrote,
/// and so that `--dry-run` shows a body a person can compare with their input. The API
/// treats `questions` as a map, where order carries no meaning.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationRequest {
    state: State,
    model: ModelId,
    questions: Vec<(QuestionId, Question)>,
    images: Vec<EmbeddedImage>,
    videos: Vec<EmbeddedVideo>,
    reject_if_busy: bool,
    keep_alive: Option<serde_json::Value>,
    max_length: Option<u32>,
    max_state_tokens: Option<u32>,
    media_kwargs: Option<serde_json::Value>,
}

impl EvaluationRequest {
    /// Assembles a request.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::NoQuestions`] for an empty question list, and
    /// [`RequestError::DuplicateQuestionId`] if two questions share an id — which would
    /// otherwise collapse silently into one JSON key and drop a question the user paid
    /// to ask.
    pub fn new(
        state: State,
        model: ModelId,
        questions: Vec<(QuestionId, Question)>,
    ) -> Result<Self, RequestError> {
        if questions.len() < MIN_QUESTIONS {
            return Err(RequestError::NoQuestions);
        }
        let mut seen = BTreeSet::new();
        for (id, _) in &questions {
            if !seen.insert(id.as_str()) {
                return Err(RequestError::DuplicateQuestionId {
                    id: id.as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            state,
            model,
            questions,
            images: Vec::new(),
            videos: Vec::new(),
            reject_if_busy: false,
            keep_alive: None,
            max_length: None,
            max_state_tokens: None,
            media_kwargs: None,
        })
    }

    /// The state all questions refer to.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// The model the request asks for.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// The questions, in the order they were supplied.
    #[must_use]
    pub fn questions(&self) -> &[(QuestionId, Question)] {
        &self.questions
    }

    /// How many questions the request carries.
    #[must_use]
    pub fn question_count(&self) -> usize {
        self.questions.len()
    }

    /// Attaches validated images shared by every question, in supplied order.
    ///
    /// # Errors
    /// Returns [`MediaError`] if the request exceeds four images, 8 MiB compressed
    /// bytes, or 64 million header pixels shared with any attached video frames.
    pub fn with_images(mut self, images: Vec<EmbeddedImage>) -> Result<Self, MediaError> {
        if images.len() > MAX_IMAGES {
            return Err(MediaError::TooManyImages);
        }
        validate_media_bytes(&images, &self.videos)?;
        self.images = images;
        Ok(self)
    }

    /// Images shared by all questions, in request order.
    #[must_use]
    pub fn images(&self) -> &[EmbeddedImage] {
        &self.images
    }

    /// Attaches explicit local videos, sharing the 8 MiB budget with still images.
    ///
    /// # Errors
    /// Returns [`MediaError`] for more than four videos, over 8 MiB combined
    /// compressed bytes, or over 64 million total header pixels.
    pub fn with_videos(mut self, videos: Vec<EmbeddedVideo>) -> Result<Self, MediaError> {
        if videos.len() > MAX_VIDEOS {
            return Err(MediaError::TooManyVideos);
        }
        validate_media_bytes(&self.images, &videos)?;
        self.videos = videos;
        Ok(self)
    }

    /// Explicit videos shared by every question, in request order.
    #[must_use]
    pub fn videos(&self) -> &[EmbeddedVideo] {
        &self.videos
    }

    /// Sets the local Python model's context budget with a client ceiling of 65536.
    ///
    /// The publisher defaults to 16384 and truncates state after reserving schema and
    /// media tokens. This option is a Python function argument in the upstream API:
    /// <https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py>.
    ///
    /// # Errors
    /// Returns [`LocalOptionsError::MaxLength`] for zero or more than 65536 tokens.
    pub fn with_max_length(mut self, value: u32) -> Result<Self, LocalOptionsError> {
        if !(1..=65536).contains(&value) {
            return Err(LocalOptionsError::MaxLength);
        }
        self.max_length = Some(value);
        Ok(self)
    }

    /// The explicitly supplied local Python context budget.
    #[must_use]
    pub const fn max_length(&self) -> Option<u32> {
        self.max_length
    }

    /// Sets a constrained subset of documented Hugging Face processor options.
    ///
    /// `media_kwargs` is forwarded by the publisher's Python model to its processor.
    /// Pixel limits are from `Qwen2VLImageProcessor`; video sampling keys are from
    /// Transformers `VideosKwargs`. The numeric ceilings are this client's bounds:
    /// <https://github.com/huggingface/transformers/blob/main/src/transformers/models/qwen2_vl/image_processing_qwen2_vl.py>
    /// and <https://github.com/huggingface/transformers/blob/main/src/transformers/processing_utils.py>.
    ///
    /// # Errors
    /// Returns [`LocalOptionsError::MediaKwargs`] for unknown keys or invalid values.
    pub fn with_media_kwargs(
        mut self,
        value: serde_json::Value,
    ) -> Result<Self, LocalOptionsError> {
        let object = value.as_object().ok_or(LocalOptionsError::MediaKwargs)?;
        if object.contains_key("fps") && object.contains_key("num_frames") {
            return Err(LocalOptionsError::MediaKwargs);
        }
        for (key, value) in object {
            let valid = match key.as_str() {
                "min_pixels" | "max_pixels" => value
                    .as_u64()
                    .is_some_and(|pixels| (1..=16_000_000).contains(&pixels)),
                "fps" => value
                    .as_f64()
                    .is_some_and(|fps| fps.is_finite() && fps > 0.0 && fps <= 120.0),
                "num_frames" => value
                    .as_u64()
                    .is_some_and(|frames| (1..=32).contains(&frames)),
                "do_sample_frames" => value.is_boolean(),
                _ => false,
            };
            if !valid {
                return Err(LocalOptionsError::MediaKwargs);
            }
        }
        if let (Some(minimum), Some(maximum)) = (
            object.get("min_pixels").and_then(serde_json::Value::as_u64),
            object.get("max_pixels").and_then(serde_json::Value::as_u64),
        ) && minimum > maximum
        {
            return Err(LocalOptionsError::MediaKwargs);
        }
        self.media_kwargs = Some(value);
        Ok(self)
    }

    /// Bounds textual state independently of total context, including a zero-token state.
    ///
    /// # Errors
    /// Returns [`LocalOptionsError::MaxStateTokens`] above the local token ceiling.
    pub fn with_max_state_tokens(mut self, value: u32) -> Result<Self, LocalOptionsError> {
        if value > 65536 {
            return Err(LocalOptionsError::MaxStateTokens);
        }
        self.max_state_tokens = Some(value);
        Ok(self)
    }

    /// Independent textual state token limit for the local bridge.
    #[must_use]
    pub const fn max_state_tokens(&self) -> Option<u32> {
        self.max_state_tokens
    }

    /// Explicit processor settings for the local Python serving bridge.
    #[must_use]
    pub const fn media_kwargs(&self) -> Option<&serde_json::Value> {
        self.media_kwargs.as_ref()
    }

    /// Sets Cloudflare's optional capacity rejection policy.
    #[must_use]
    pub const fn with_reject_if_busy(mut self, value: bool) -> Self {
        self.reject_if_busy = value;
        self
    }

    /// Whether Cloudflare should reject unavailable capacity instead of queueing.
    #[must_use]
    pub const fn reject_if_busy(&self) -> bool {
        self.reject_if_busy
    }

    /// Sets Ollama's duration string or integer seconds model retention option.
    ///
    /// Duration syntax is interpreted by Ollama. The CLI bounds strings and refuses
    /// control characters before transmission; negative seconds mean keep loaded.
    /// See <https://docs.ollama.com/api/systemone>.
    ///
    /// # Errors
    /// Returns [`KeepAliveError`] for other JSON types, blank, or excessive strings.
    pub fn with_keep_alive(mut self, value: serde_json::Value) -> Result<Self, KeepAliveError> {
        let valid = match &value {
            serde_json::Value::String(text) => {
                !text.trim().is_empty() && text.len() <= 128 && !text.chars().any(char::is_control)
            }
            serde_json::Value::Number(number) => number.as_i64().is_some(),
            _ => false,
        };
        if !valid {
            return Err(KeepAliveError);
        }
        self.keep_alive = Some(value);
        Ok(self)
    }

    /// Ollama's explicitly supplied model retention duration or integer seconds.
    #[must_use]
    pub const fn keep_alive(&self) -> Option<&serde_json::Value> {
        self.keep_alive.as_ref()
    }
}

fn validate_media_bytes(
    images: &[EmbeddedImage],
    videos: &[EmbeddedVideo],
) -> Result<(), MediaError> {
    let total = images
        .iter()
        .map(EmbeddedImage::byte_len)
        .chain(videos.iter().map(EmbeddedVideo::byte_len))
        .try_fold(0usize, usize::checked_add)
        .ok_or(MediaError::TotalImagesTooLarge)?;
    if total > MAX_TOTAL_IMAGE_BYTES {
        return Err(MediaError::TotalImagesTooLarge);
    }
    let pixels = images
        .iter()
        .map(EmbeddedImage::pixel_len)
        .chain(videos.iter().map(EmbeddedVideo::pixel_len))
        .try_fold(0usize, usize::checked_add)
        .ok_or(MediaError::TotalMediaPixelsTooLarge)?;
    if pixels > MAX_TOTAL_MEDIA_PIXELS {
        return Err(MediaError::TotalMediaPixelsTooLarge);
    }
    Ok(())
}

impl Serialize for EvaluationRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let len = 3
            + usize::from(!self.images.is_empty())
            + usize::from(!self.videos.is_empty())
            + usize::from(self.reject_if_busy)
            + usize::from(self.keep_alive.is_some())
            + usize::from(self.max_length.is_some())
            + usize::from(self.max_state_tokens.is_some())
            + usize::from(self.media_kwargs.is_some());
        let mut object = serializer.serialize_struct("EvaluationRequest", len)?;
        object.serialize_field("state", &self.state)?;
        object.serialize_field("model", &self.model)?;
        object.serialize_field("questions", &QuestionMap(&self.questions))?;
        if !self.images.is_empty() {
            object.serialize_field("images", &self.images)?;
        }
        if !self.videos.is_empty() {
            object.serialize_field("videos", &self.videos)?;
        }
        if let Some(value) = self.max_state_tokens {
            object.serialize_field("max_state_tokens", &value)?;
        }
        if let Some(max_length) = self.max_length {
            object.serialize_field("max_length", &max_length)?;
        }
        if let Some(media_kwargs) = &self.media_kwargs {
            object.serialize_field("media_kwargs", media_kwargs)?;
        }
        if self.reject_if_busy {
            object.serialize_field("options", &serde_json::json!({"rejectIfBusy":true}))?;
        }
        if let Some(keep_alive) = &self.keep_alive {
            object.serialize_field("keep_alive", keep_alive)?;
        }
        object.end()
    }
}

struct QuestionMap<'a>(&'a [(QuestionId, Question)]);

impl Serialize for QuestionMap<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, question) in self.0 {
            map.serialize_entry(id.as_str(), question)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::question::ChoiceOption;

    fn text(value: &str) -> Content {
        Content::text(value).unwrap()
    }

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).unwrap()
    }

    #[test]
    fn wire_form_matches_the_api_reference_example() {
        // Transcribed from https://docs.typesafe.ai/api, "Example request".
        let request = EvaluationRequest::new(
            State::text("Help! My payouts have been failing for 3 days.").unwrap(),
            ModelId::default(),
            vec![(
                id("is_urgent"),
                Question::noul(text("Does this convey urgency?"), None).unwrap(),
            )],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "state": "Help! My payouts have been failing for 3 days.",
                "model": "jev-latest",
                "questions": {
                    "is_urgent": {"type": "noul", "instructions": "Does this convey urgency?"}
                }
            })
        );
    }

    #[test]
    fn a_mixed_request_carries_every_primitive() {
        let request = EvaluationRequest::new(
            State::text("ticket text").unwrap(),
            ModelId::new("jev-1.13.0").unwrap(),
            vec![
                (id("urgent"), Question::noul(text("Urgent?"), None).unwrap()),
                (
                    id("team"),
                    Question::choice(
                        text("Which team?"),
                        vec![
                            ChoiceOption::new("billing", None).unwrap(),
                            ChoiceOption::new("technical", None).unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                (
                    id("severity"),
                    Question::score(text("How severe?"), vec![text("low"), text("high")]).unwrap(),
                ),
            ],
        )
        .unwrap();

        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["model"], json!("jev-1.13.0"));
        assert_eq!(encoded["questions"]["urgent"]["type"], json!("noul"));
        assert_eq!(encoded["questions"]["team"]["type"], json!("choice"));
        assert_eq!(encoded["questions"]["severity"]["type"], json!("score"));
        assert_eq!(request.question_count(), 3);
    }

    #[test]
    fn structured_state_is_preserved_exactly() {
        let state = State::new(Content::try_from(json!({"subject": "x", "body": ["y"]})).unwrap());
        let request = EvaluationRequest::new(
            state,
            ModelId::default(),
            vec![(id("q"), Question::noul(text("?"), None).unwrap())],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&request).unwrap()["state"],
            json!({"subject": "x", "body": ["y"]})
        );
    }

    #[test]
    fn rejects_no_questions() {
        assert_eq!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), Vec::new()),
            Err(RequestError::NoQuestions)
        );
    }

    #[test]
    fn rejects_duplicate_question_ids() {
        // Without this, the second question silently overwrites the first in the JSON
        // object and the user is billed for a question they never get an answer to.
        let questions = vec![
            (id("same"), Question::noul(text("a"), None).unwrap()),
            (id("same"), Question::noul(text("b"), None).unwrap()),
        ];
        assert_eq!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), questions),
            Err(RequestError::DuplicateQuestionId {
                id: "same".to_owned()
            })
        );
    }

    #[test]
    fn question_ids_are_trimmed_so_whitespace_cannot_hide_a_duplicate() {
        assert_eq!(QuestionId::new(" urgent ").unwrap().as_str(), "urgent");
        // And the trimmed forms collide, so the duplicate check sees them.
        let questions = vec![
            (
                QuestionId::new(" same").unwrap(),
                Question::noul(text("a"), None).unwrap(),
            ),
            (
                QuestionId::new("same ").unwrap(),
                Question::noul(text("b"), None).unwrap(),
            ),
        ];
        assert!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), questions)
                .is_err()
        );
    }

    #[test]
    fn question_ids_reject_blank_and_control_characters() {
        assert_eq!(QuestionId::new(" "), Err(QuestionIdError::Empty));
        assert_eq!(
            QuestionId::new("a\u{1b}b"),
            Err(QuestionIdError::ControlCharacter)
        );
        assert!(QuestionId::new("is_urgent").is_ok());
    }

    #[test]
    fn empty_state_is_refused_before_it_costs_anything() {
        assert!(State::text("").is_err());
        assert!(State::text("   \t\n").is_err());
    }
}
