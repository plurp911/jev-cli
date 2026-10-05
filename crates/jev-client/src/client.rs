//! Request construction, retry orchestration, and response dispatch.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use jev_core::{EvaluationRequest, EvaluationResponse, ModelCard};

use crate::credential::Credential;
use crate::endpoint::{Endpoint, MODELS_PATH, SYSTEM_ONE_PATH};
use crate::error::{ClientError, TransportError};
use crate::retry::{Outcome, RetryDecision, RetryPolicy};
use crate::transport::{Request, Response, Transport};
use crate::wire;

/// The `User-Agent` this CLI identifies itself with.
///
/// A distinct agent string lets TypeSafe distinguish this community tool from their own
/// SDKs in their logs, which matters if `jev` ever misbehaves at scale. It carries the
/// version and nothing about the user or the machine.
#[must_use]
pub fn user_agent() -> String {
    format!("jev-cli/{}", env!("CARGO_PKG_VERSION"))
}

/// Something that can wait and tell the time.
///
/// Injected so the retry loop can be tested at full speed with no sleeping, which is
/// what keeps the suite deterministic (`AGENTS.md` §9).
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
    /// Blocks for `duration`.
    fn sleep(&self, duration: Duration);
    /// A jitter sample in `[0, 1)`.
    fn jitter_sample(&self) -> f64;

    /// Whether the caller has been asked to stop.
    ///
    /// Consulted after every wait, so an interrupt that arrives during a backoff ends
    /// the loop instead of being noticed only after the remaining attempts have run.
    /// `jev-client` has no signal handling of its own — it cannot, without becoming the
    /// process — so the answer comes from the caller, which does.
    ///
    /// The default is `false`, which is exactly the old behaviour for every clock that
    /// has nothing to report.
    fn cancelled(&self) -> bool {
        false
    }
}

/// The real clock: `std::time` and `std::thread::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    /// A cheap, non-cryptographic jitter source.
    ///
    /// Derived from the current nanosecond, which is enough to decorrelate retries
    /// between processes. It is deliberately not a random-number-generator dependency:
    /// jitter is not a security property, and the official SDK uses an ordinary PRNG
    /// for the same purpose.
    fn jitter_sample(&self) -> f64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        f64::from(nanos) / 1_000_000_000.0
    }
}

/// What happened during one API call, for diagnostics.
///
/// Carries no request or response content — only shape, timing, and the API's own
/// identifier for the call — so that printing it cannot disclose the user's state or
/// the API's reply.
///
/// Not `Copy`: `request_id` is an owned `String`. It is small and cloned once per call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallStats {
    /// How many HTTP attempts were made, including the first.
    pub attempts: u32,
    /// Wall-clock time for the whole call, including waits between attempts.
    pub elapsed: Duration,
    /// The final HTTP status, when a response arrived.
    pub status: Option<u16>,
    /// The API's own identifier for the final attempt, from `x-typesafe-request-id`.
    ///
    /// Documented at <https://docs.typesafe.ai/sdk/python/api/exceptions.md>, where the
    /// official SDK exposes it as `TypeSafeAPIError.request_id` and appends it to every
    /// API error message. It is an opaque support identifier, not credential material,
    /// and it is the one thing TypeSafe can use to find a specific call — without it a
    /// user whose batch row failed has nothing to hand support.
    ///
    /// `None` when no response arrived, or when the API did not send the header.
    pub request_id: Option<String>,
}

/// A System One API client.
///
/// The client holds no credential: one is supplied per call. That keeps a secret's
/// lifetime as short as the request that needs it, and means a `Client` can be built,
/// inspected, and passed around in tests without holding one.
#[derive(Debug)]
pub struct Client<T: Transport, C: Clock = SystemClock> {
    transport: T,
    clock: C,
    endpoint: Endpoint,
    retry: RetryPolicy,
}

impl<T: Transport> Client<T> {
    /// Builds a client against `endpoint` with the default retry policy and the real
    /// clock.
    pub fn new(transport: T, endpoint: Endpoint) -> Self {
        Self {
            transport,
            clock: SystemClock,
            endpoint,
            retry: RetryPolicy::default(),
        }
    }
}

impl<T: Transport, C: Clock> Client<T, C> {
    /// Builds a client with an explicit clock, for tests.
    pub const fn with_clock(transport: T, endpoint: Endpoint, clock: C) -> Self {
        Self {
            transport,
            clock,
            endpoint,
            retry: RetryPolicy {
                max_retries: 2,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(5),
                jitter: 0.25,
                respect_retry_after: true,
                total_budget: Duration::from_secs(30),
            },
        }
    }

    /// Replaces the retry policy.
    #[must_use]
    pub const fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The endpoint this client talks to.
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The retry policy in force.
    pub const fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Builds the exact HTTP request an evaluation would send.
    ///
    /// Exposed so that `--dry-run` shows the user the real bytes rather than a
    /// reconstruction. The credential is not part of a [`Request`], so this value is
    /// safe to print in full.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::InvalidRequest`] if the selected provider cannot accept
    /// or encode this request.
    pub fn build_evaluation_request(
        &self,
        request: &EvaluationRequest,
    ) -> Result<Request, ClientError> {
        build_evaluation_request(&self.endpoint, request)
    }

    /// Evaluates a request.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] for a transport failure, a non-2xx status the retry
    /// policy did not resolve, or a response this version cannot decode.
    pub fn evaluate(
        &self,
        request: &EvaluationRequest,
        credential: &Credential,
    ) -> (Result<EvaluationResponse, ClientError>, CallStats) {
        let http = match self.build_evaluation_request(request) {
            Ok(http) => http,
            Err(error) => return (Err(error), CallStats::default()),
        };
        let (outcome, stats) = self.send(&http, credential, request.reject_if_busy());
        (
            outcome.and_then(|response| {
                if self.endpoint.is_cloudflare() {
                    wire::decode_cloudflare_evaluation(&response.body)
                } else if self.endpoint.provider() == "huggingface" {
                    wire::decode_publisher_evaluation(&response.body)
                } else {
                    wire::decode_evaluation(&response.body)
                }
            }),
            stats,
        )
    }

    /// Lists models exposed by the selected provider.
    ///
    /// Cloudflare returns the two documented Clef models after a successful catalog
    /// search. The upstream search API does not specify the shape of its entries.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] as [`Client::evaluate`] does.
    pub fn models(
        &self,
        credential: &Credential,
    ) -> (Result<Vec<ModelCard>, ClientError>, CallStats) {
        let path = match self.endpoint.provider() {
            "cloudflare" => format!(
                "/client/v4/accounts/{}/ai/models/search?search=clef",
                self.endpoint.cloudflare_account_id().unwrap_or_default()
            ),
            "ollama" => "/api/tags".to_owned(),
            _ => MODELS_PATH.to_owned(),
        };
        let http = Request {
            url: self.endpoint.url_for(&path),
            method: "GET",
            headers: json_headers(false),
            body: Vec::new(),
        };
        let (outcome, stats) = self.send(&http, credential, false);
        (
            outcome.and_then(|response| match self.endpoint.provider() {
                "cloudflare" => wire::decode_cloudflare_models(&response.body),
                "ollama" => wire::decode_local_models(&response.body, true),
                "llamacpp" => wire::decode_local_models(&response.body, false),
                _ => wire::decode_models(&response.body),
            }),
            stats,
        )
    }

    /// Runs one logical call: attempt, classify, wait, repeat within the budget.
    fn send(
        &self,
        request: &Request,
        credential: &Credential,
        reject_if_busy: bool,
    ) -> (Result<Response, ClientError>, CallStats) {
        let started = self.clock.now();
        let anonymous = Credential::anonymous();
        let credential = if self.endpoint.is_local_provider() && self.endpoint.is_loopback() {
            &anonymous
        } else {
            credential
        };
        let mut attempt: u32 = 0;
        let mut last: Result<Response, TransportError>;

        loop {
            attempt = attempt.saturating_add(1);
            last = self.transport.execute(request, credential);

            // Explicit capacity rejection asks for a prompt answer, so retrying the
            // documented rejection would defeat that policy.
            if self.endpoint.is_cloudflare()
                && reject_if_busy
                && last.as_ref().is_ok_and(|response| {
                    response.status == 429
                        && wire::cloudflare_error_code(&response.body) == Some(3040)
                })
            {
                break;
            }

            let elapsed = self.clock.now().saturating_duration_since(started);
            let outcome = match &last {
                Ok(response) => Outcome::Status(response),
                Err(error) => Outcome::Transport(error),
            };

            match self
                .retry
                .decide(attempt, outcome, elapsed, self.clock.jitter_sample())
            {
                RetryDecision::Stop => break,
                RetryDecision::RetryAfter(delay) => {
                    self.clock.sleep(delay);
                    // A real clock's `sleep` returns early when the caller has been
                    // interrupted. Without this check the loop would simply start the
                    // next attempt, so Ctrl-C during a five-second `Retry-After` was
                    // swallowed: the process ran every remaining attempt and exited 4,
                    // while `main.rs` promised it would stop and report 130.
                    if self.clock.cancelled() {
                        break;
                    }
                }
            }
        }

        let elapsed = self.clock.now().saturating_duration_since(started);
        let final_status = last.as_ref().ok().map(|response| response.status);
        // From the attempt that actually ended the call, which is the one support will
        // be asked about.
        let request_id = last
            .as_ref()
            .ok()
            .and_then(|response| {
                response.header(if self.endpoint.is_cloudflare() {
                    "cf-ray"
                } else {
                    REQUEST_ID_HEADER
                })
            })
            .map(|id| redact(id, credential));
        let stats = CallStats {
            attempts: attempt,
            elapsed,
            status: final_status,
            request_id,
        };

        let result = match last {
            Err(error) => Err(ClientError::Transport(error)),
            Ok(response) if (200..300).contains(&response.status) => Ok(response),
            Ok(response) => Err(ClientError::from_status(
                response.status,
                // Redacted in full and only then truncated (by `from_status`): redacting
                // a clipped message left the head of a key that straddled the limit.
                if self.endpoint.is_cloudflare() {
                    Some(wire::cloudflare_error_code(&response.body).map_or_else(
                        || "Cloudflare rejected the request".to_owned(),
                        |code| format!("Cloudflare error code {code}"),
                    ))
                } else if self.endpoint.is_local_provider() {
                    Some(format!(
                        "{} rejected the request; check server configuration and model availability",
                        self.endpoint.provider()
                    ))
                } else {
                    wire::extract_error_text(&response.body)
                        .map(|message| redact(&message, credential))
                },
            )),
        };
        (result, stats)
    }
}

/// Builds the exact HTTP request an evaluation would send, without a client.
///
/// [`Client::build_evaluation_request`] delegates here, and so does `--dry-run`, which
/// has no transport to construct a [`Client`] around. One function means a dry run
/// cannot describe a request that differs from the one a real run would send: the URL,
/// the method, the header set, and the body bytes all come from here in both cases.
///
/// The credential is not part of a [`Request`], so the result is safe to print in full.
///
/// # Errors
///
/// Returns [`ClientError::InvalidRequest`] if the selected provider cannot accept
/// or encode this request.
pub fn build_evaluation_request(
    endpoint: &Endpoint,
    request: &EvaluationRequest,
) -> Result<Request, ClientError> {
    evaluation_request(endpoint, request, false)
}

/// Validates the complete request and returns its exact encoded body size.
///
/// Media base64 consists only of unescaped ASCII. Counting its canonical length
/// avoids repeatedly encoding batch template images before anything is sent.
///
/// # Errors
/// Returns [`ClientError::InvalidRequest`] for unsupported input or provider limits.
pub fn preflight_evaluation_request(
    endpoint: &Endpoint,
    request: &EvaluationRequest,
) -> Result<usize, ClientError> {
    let http = evaluation_request(endpoint, request, true)?;
    Ok(http.body.len().saturating_add(media_base64_len(request)))
}

fn media_base64_len(request: &EvaluationRequest) -> usize {
    request
        .images()
        .iter()
        .chain(
            request
                .videos()
                .iter()
                .flat_map(jev_core::EmbeddedVideo::frames),
        )
        .map(|image| image.byte_len().div_ceil(3) * 4)
        .sum()
}

fn evaluation_request(
    endpoint: &Endpoint,
    request: &EvaluationRequest,
    sizing: bool,
) -> Result<Request, ClientError> {
    let provider = endpoint.provider();
    if provider != "huggingface" && publisher_content(request) {
        return Err(invalid_request(
            "scalar or blank publisher JSON content requires the local Python Clef bridge",
        ));
    }
    if (!request.videos().is_empty()
        || request.max_length().is_some()
        || request.max_state_tokens().is_some()
        || request.media_kwargs().is_some())
        && provider != "huggingface"
    {
        return Err(invalid_request(
            "videos, max-length, max-state-tokens, and media kwargs require the local Python Clef bridge",
        ));
    }
    if request.reject_if_busy() && !endpoint.is_cloudflare() {
        return Err(invalid_request(
            "reject-if-busy is supported only by Cloudflare",
        ));
    }
    if !request.images().is_empty() && !matches!(provider, "cloudflare" | "ollama" | "huggingface")
    {
        return Err(invalid_request(
            "images require Cloudflare, Ollama, or the local Python Clef bridge",
        ));
    }
    if matches!(provider, "cloudflare" | "ollama" | "huggingface") && request.question_count() > 64
    {
        return Err(invalid_request(
            "this provider accepts at most 64 questions",
        ));
    }
    if endpoint.is_cloudflare()
        && request.questions().iter().any(|(id, _)| {
            id.as_str().len() > 100
                || !id
                    .as_str()
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        })
    {
        return Err(invalid_request(
            "Cloudflare question ids must be at most 100 ASCII letters, digits, underscores, periods, or hyphens",
        ));
    }
    let max_score = match provider {
        "huggingface" => jev_core::limits::SCORE_ABSOLUTE_MAX_LEVELS,
        "ollama" => jev_core::limits::OLLAMA_SCORE_MAX_LEVELS,
        _ => jev_core::limits::SCORE_MAX_LEVELS,
    };
    for (_, question) in request.questions() {
        match question {
            jev_core::Question::Choice { options, .. }
                if provider == "ollama" && options.len() > 26 =>
            {
                return Err(invalid_request("Ollama accepts at most 26 Choice options"));
            }
            jev_core::Question::Score { levels, .. } if levels.len() > max_score => {
                return Err(invalid_request(
                    "the Score rubric exceeds this provider's level limit",
                ));
            }
            _ => {}
        }
    }
    if request.keep_alive().is_some() && provider != "ollama" {
        return Err(invalid_request("keep-alive is supported only by Ollama"));
    }
    if provider == "huggingface" {
        validate_bridge_options(request)?;
    }
    let model = normalized_model(endpoint, request.model().as_str())?;
    let path = endpoint.cloudflare_account_id().map_or_else(
        || SYSTEM_ONE_PATH.to_owned(),
        |account| format!("/client/v4/accounts/{account}/ai/run/@cf/cloudflare/{model}"),
    );
    let body = serde_json::to_vec(&EvaluationBody {
        request,
        model,
        ollama: provider == "ollama",
        sizing,
    })
    .map_err(|_| invalid_request("could not encode the request body"))?;
    let limit = request_body_limit(provider, !request.images().is_empty());
    let body_len = body
        .len()
        .saturating_add(if sizing { media_base64_len(request) } else { 0 });
    if limit.is_some_and(|limit| body_len > limit) {
        return Err(invalid_request(
            "the encoded request body exceeds this provider's byte limit",
        ));
    }
    Ok(Request {
        url: endpoint.url_for(&path),
        method: "POST",
        headers: json_headers(true),
        body,
    })
}

fn request_body_limit(provider: &str, images: bool) -> Option<usize> {
    match provider {
        "cloudflare" | "huggingface" => Some(13 * 1024 * 1024),
        "ollama" if !images => Some(64 * 1024),
        "ollama" => Some(32 * 1024 * 1024),
        _ => None,
    }
}

fn publisher_content(request: &EvaluationRequest) -> bool {
    request.state().content().is_local_json()
        || request.questions().iter().any(|(_, question)| {
            question.instructions().is_local_json()
                || match question {
                    jev_core::Question::Noul { criteria, .. } => {
                        criteria.as_ref().is_some_and(|criteria| {
                            [criteria.yes(), criteria.no()]
                                .into_iter()
                                .flatten()
                                .any(jev_core::Content::is_local_json)
                        })
                    }
                    jev_core::Question::Choice { options, .. } => options
                        .iter()
                        .filter_map(jev_core::ChoiceOption::description)
                        .any(jev_core::Content::is_local_json),
                    jev_core::Question::Score { levels, .. } => {
                        levels.iter().any(jev_core::Content::is_local_json)
                    }
                }
        })
}

fn normalized_model<'a>(endpoint: &Endpoint, model: &'a str) -> Result<&'a str, ClientError> {
    if !endpoint.is_cloudflare() {
        return Ok(model);
    }
    match model {
        "clef" | "@cf/cloudflare/clef" => Ok("clef"),
        "clef-flash" | "@cf/cloudflare/clef-flash" => Ok("clef-flash"),
        _ => Err(invalid_request(
            "Cloudflare model must be clef or clef-flash",
        )),
    }
}

fn sparse_video_metadata(video: &jev_core::EmbeddedVideo) -> bool {
    video.metadata().is_some_and(|metadata| {
        let count = video.frames().len();
        metadata
            .get("total_num_frames")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|total| usize::try_from(total).ok() != Some(count))
            || metadata
                .get("frames_indices")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|indices| {
                    indices.iter().enumerate().any(|(index, value)| {
                        value.as_u64().and_then(|value| usize::try_from(value).ok()) != Some(index)
                    })
                })
    })
}

fn validate_bridge_options(request: &EvaluationRequest) -> Result<(), ClientError> {
    if request
        .questions()
        .iter()
        .any(|(id, _)| id.as_str().chars().count() > 1024)
    {
        return Err(invalid_request(
            "local Python question ids must contain at most 1024 Unicode characters",
        ));
    }
    let kwargs = request.media_kwargs();
    let sampling = kwargs
        .and_then(|value| value.get("do_sample_frames"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if sampling && request.videos().iter().any(sparse_video_metadata) {
        return Err(invalid_request(
            "sparse source video metadata requires do_sample_frames=false; resampling would discard source timestamps",
        ));
    }
    let sampled_frames = kwargs
        .and_then(|value| value.get("num_frames"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(32);
    if sampling && sampled_frames == 1 && !request.videos().is_empty() {
        return Err(invalid_request(
            "the local video processor needs at least two sampled frames",
        ));
    }
    let pixels = request
        .images()
        .iter()
        .map(jev_core::EmbeddedImage::pixel_len)
        .sum::<usize>()
        + request
            .videos()
            .iter()
            .map(|video| {
                let padding = if video.frames().len() == 1 {
                    video
                        .frames()
                        .first()
                        .map_or(0, jev_core::EmbeddedImage::pixel_len)
                } else {
                    0
                };
                video.pixel_len() + padding
            })
            .sum::<usize>();
    if pixels > 64_000_000 {
        return Err(invalid_request(
            "padding video frames exceeds the total media pixel budget",
        ));
    }
    if request
        .images()
        .iter()
        .chain(
            request
                .videos()
                .iter()
                .flat_map(jev_core::EmbeddedVideo::frames),
        )
        .any(|image| {
            image.content_type() == "image/webp"
                && image.bytes().get(12..16) == Some(b"VP8X")
                && image.bytes().get(20).is_some_and(|flags| flags & 2 != 0)
        })
    {
        return Err(invalid_request(
            "the local Python bridge does not accept animated WebP",
        ));
    }
    validate_bridge_allocation(request, sampling, sampled_frames)
}

fn validate_bridge_allocation(
    request: &EvaluationRequest,
    sampling: bool,
    sampled_frames: usize,
) -> Result<(), ClientError> {
    let kwargs = request.media_kwargs();
    let item_count = request.images().len() + request.videos().len();
    let budget = 64_000_000_usize
        .checked_div(item_count.max(1))
        .unwrap_or(64_000_000);
    let control = |key: &str| {
        kwargs
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    if [control("max_pixels"), control("min_pixels")]
        .into_iter()
        .flatten()
        .any(|value| usize::try_from(value).unwrap_or(usize::MAX) > budget)
    {
        return Err(invalid_request(
            "processor resize controls exceed the total media pixel budget",
        ));
    }
    let image_maximum = control("max_pixels")
        .unwrap_or(u32::try_from(budget.min(16_000_000)).unwrap_or(16_000_000));
    let video_maximum = control("max_pixels")
        .unwrap_or(u32::try_from(budget.min(25_165_824)).unwrap_or(25_165_824));
    let image_minimum = control("min_pixels").unwrap_or(65_536.min(image_maximum));
    let video_minimum = control("min_pixels").unwrap_or(4096.min(video_maximum));
    let mut processed_total = 0.0;
    for image in request.images() {
        let (frame, total) = resized_pixels(image, 1, image_minimum, image_maximum, false);
        if frame > 16_000_000.0 {
            return Err(invalid_request(
                "processor resize controls exceed the per-frame pixel budget",
            ));
        }
        processed_total += total;
    }
    for video in request.videos() {
        let count = if sampling {
            video.frames().len().max(sampled_frames).max(2)
        } else {
            video.frames().len().max(2)
        };
        let first = if sampling { 2 } else { count };
        let mut clip_total: f64 = 0.0;
        if let Some(image) = video.frames().first() {
            for count in first..=count {
                let (frame, total) =
                    resized_pixels(image, count, video_minimum, video_maximum, true);
                if frame > 16_000_000.0 {
                    return Err(invalid_request(
                        "processor resize controls exceed the per-frame pixel budget",
                    ));
                }
                clip_total = clip_total.max(total);
            }
        }
        processed_total += clip_total;
    }
    if processed_total > 64_000_000.0 {
        return Err(invalid_request(
            "processor resize controls exceed the total media pixel budget",
        ));
    }
    Ok(())
}

// Allocation estimate for the pinned Qwen image/video processors, before decoding.
// Video limits cover a clip; spatial patches and temporal padding can exceed a
// nominal area limit, so compare their quantized allocation against our ceilings.
fn resized_pixels(
    image: &jev_core::EmbeddedImage,
    count: usize,
    minimum: u32,
    maximum: u32,
    temporal: bool,
) -> (f64, f64) {
    let width = f64::from(u32::try_from(image.width()).unwrap_or(u32::MAX));
    let height = f64::from(u32::try_from(image.height()).unwrap_or(u32::MAX));
    let frames = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
    let padded = if temporal { count + count % 2 } else { count };
    let padded = f64::from(u32::try_from(padded).unwrap_or(u32::MAX));
    let mut width_patches = (width / 32.0).round_ties_even();
    let mut height_patches = (height / 32.0).round_ties_even();
    let rounded = width_patches * height_patches * 1024.0 * padded;
    let area = frames * width * height;
    if rounded > f64::from(maximum) {
        let divisor = (area / f64::from(maximum)).sqrt();
        width_patches = (width / divisor / 32.0).floor().max(1.0);
        height_patches = (height / divisor / 32.0).floor().max(1.0);
    } else if rounded < f64::from(minimum) {
        let multiplier = (f64::from(minimum) / area).sqrt();
        width_patches = (width * multiplier / 32.0).ceil();
        height_patches = (height * multiplier / 32.0).ceil();
    }
    let frame = width_patches * height_patches * 1024.0;
    (frame, padded * frame)
}

/// Serialize borrowed domain values directly so that question and option order is
/// preserved. Converting through a JSON Value would sort the map keys.
struct EvaluationBody<'a> {
    request: &'a EvaluationRequest,
    model: &'a str,
    ollama: bool,
    sizing: bool,
}

impl serde::Serialize for EvaluationBody<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("state", self.request.state())?;
        map.serialize_entry("model", self.model)?;
        map.serialize_entry("questions", &OrderedQuestions(self.request.questions()))?;
        if !self.request.images().is_empty() {
            if self.sizing && self.ollama {
                map.serialize_entry("images", &vec![""; self.request.images().len()])?;
            } else if self.sizing {
                let images: Vec<_> = self.request.images().iter().map(ImageSize::new).collect();
                map.serialize_entry("images", &images)?;
            } else if self.ollama {
                let images: Vec<String> = self
                    .request
                    .images()
                    .iter()
                    .map(jev_core::EmbeddedImage::base64)
                    .collect();
                map.serialize_entry("images", &images)?;
            } else {
                map.serialize_entry("images", self.request.images())?;
            }
        }
        if !self.request.videos().is_empty() {
            if self.sizing {
                let videos: Vec<_> = self
                    .request
                    .videos()
                    .iter()
                    .map(|video| VideoSize {
                        frames: video.frames().iter().map(ImageSize::new).collect(),
                        metadata: video.metadata(),
                    })
                    .collect();
                map.serialize_entry("videos", &videos)?;
            } else {
                map.serialize_entry("videos", self.request.videos())?;
            }
        }
        if let Some(value) = self.request.max_state_tokens() {
            map.serialize_entry("max_state_tokens", &value)?;
        }
        if let Some(max_length) = self.request.max_length() {
            map.serialize_entry("max_length", &max_length)?;
        }
        if let Some(media_kwargs) = self.request.media_kwargs() {
            map.serialize_entry("media_kwargs", media_kwargs)?;
        }
        if self.request.reject_if_busy() {
            map.serialize_entry("options", &serde_json::json!({"rejectIfBusy":true}))?;
        }
        if let Some(keep_alive) = self.request.keep_alive() {
            map.serialize_entry("keep_alive", keep_alive)?;
        }
        map.end()
    }
}

#[derive(serde::Serialize)]
struct ImageSize<'a> {
    content_type: &'a str,
    base64: &'static str,
}
impl<'a> ImageSize<'a> {
    fn new(image: &'a jev_core::EmbeddedImage) -> Self {
        Self {
            content_type: image.content_type(),
            base64: "",
        }
    }
}
#[derive(serde::Serialize)]
struct VideoSize<'a> {
    frames: Vec<ImageSize<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a serde_json::Value>,
}

struct OrderedQuestions<'a>(&'a [(jev_core::QuestionId, jev_core::Question)]);

impl serde::Serialize for OrderedQuestions<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, question) in self.0 {
            map.serialize_entry(id.as_str(), question)?;
        }
        map.end()
    }
}

fn invalid_request(message: &str) -> ClientError {
    ClientError::InvalidRequest {
        status: 400,
        message: Some(message.to_owned()),
    }
}

/// The response header carrying the API's identifier for a call.
///
/// From <https://docs.typesafe.ai/sdk/python/api/exceptions.md>. Lowercased because
/// `Transport` implementations lowercase header names.
pub const REQUEST_ID_HEADER: &str = "x-typesafe-request-id";

/// The non-credential headers every request carries.
fn json_headers(has_body: bool) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    headers.insert("accept".to_owned(), "application/json".to_owned());
    headers.insert("user-agent".to_owned(), user_agent());
    if has_body {
        headers.insert("content-type".to_owned(), "application/json".to_owned());
    }
    headers
}

/// Removes the credential from API-supplied text before it can be shown to anyone.
///
/// An error body is untrusted, and an endpoint -- a misbehaving proxy, a custom host, a
/// debugging echo server -- can quote the `Authorization` header back. That text becomes
/// an error message, which the CLI prints to a terminal and `jev mcp serve` returns into
/// an agent's context window. Neither may carry the key (`AGENTS.md` §2), so it is
/// replaced here, below both of them, in the whole decoded message before it is clipped,
/// so neither a JSON-escaped key nor one straddling the length limit survives. The
/// request-id header is untrusted text too and is redacted the same way.
fn redact(message: &str, credential: &Credential) -> String {
    let secret = credential.expose();
    if secret.is_empty() {
        return message.to_owned();
    }
    message.replace(secret, "<redacted>")
}

#[cfg(test)]
mod tests {
    #[test]
    fn score_limits_are_checked_before_transport_for_each_provider() {
        for count in [10, 11, 26, 27, 255] {
            let request = EvaluationRequest::new(
                State::text("state").unwrap(),
                ModelId::new("clef").unwrap(),
                vec![(
                    QuestionId::new("q").unwrap(),
                    Question::score_with_max(
                        Content::text("rating?").unwrap(),
                        vec![Content::text("level").unwrap(); count],
                        255,
                    )
                    .unwrap(),
                )],
            )
            .unwrap();
            for (endpoint, maximum) in [
                (Endpoint::huggingface(), 255),
                (Endpoint::ollama(), 26),
                (Endpoint::official(), 10),
                (Endpoint::llama_cpp(), 10),
                (cloudflare_endpoint(), 10),
            ] {
                assert_eq!(
                    build_evaluation_request(&endpoint, &request).is_ok(),
                    count <= maximum,
                    "{} with {count} levels",
                    endpoint.provider()
                );
                if count > maximum {
                    let transport = MockTransport::new();
                    let client = Client::new(transport, endpoint);
                    let (result, stats) = client.evaluate(&request, &credential());
                    assert!(matches!(result, Err(ClientError::InvalidRequest { .. })));
                    assert_eq!(stats.attempts, 0);
                    assert!(client.transport.observed().is_empty());
                }
            }
        }
        assert!(
            Question::score_with_max(
                Content::text("rating?").unwrap(),
                vec![Content::text("level").unwrap(); 256],
                256
            )
            .is_err()
        );
    }

    #[test]
    fn publisher_json_in_every_content_position_is_refused_by_other_adapters() {
        let strict = Content::text("question?").unwrap();
        let scalar = Content::local_json(json!(false)).unwrap();
        let questions = [
            Question::noul(strict.clone(), None).unwrap(),
            Question::noul(scalar.clone(), None).unwrap(),
            Question::noul(
                strict.clone(),
                Some(jev_core::NoulCriteria::new(Some(scalar.clone()), None).unwrap()),
            )
            .unwrap(),
            Question::noul(
                strict.clone(),
                Some(jev_core::NoulCriteria::new(None, Some(scalar.clone())).unwrap()),
            )
            .unwrap(),
            Question::choice(
                strict.clone(),
                vec![
                    jev_core::ChoiceOption::new("a", Some(scalar.clone())).unwrap(),
                    jev_core::ChoiceOption::new("b", None).unwrap(),
                ],
            )
            .unwrap(),
            Question::score(strict.clone(), vec![strict, scalar.clone()]).unwrap(),
        ];
        for (index, question) in questions.into_iter().enumerate() {
            let state = if index == 0 {
                State::new(scalar.clone())
            } else {
                State::text("state").unwrap()
            };
            let request = EvaluationRequest::new(
                state,
                ModelId::new("clef").unwrap(),
                vec![(QuestionId::new("q").unwrap(), question)],
            )
            .unwrap();
            assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_ok());
            for endpoint in [
                Endpoint::official(),
                Endpoint::ollama(),
                Endpoint::llama_cpp(),
                cloudflare_endpoint(),
            ] {
                let error = build_evaluation_request(&endpoint, &request).unwrap_err();
                assert!(
                    error.to_string().contains("publisher JSON content"),
                    "position {index}: {error}"
                );
            }
        }
    }

    #[test]
    fn python_question_id_limit_counts_unicode_characters_before_sending() {
        for (id, accepted) in [
            ("x".repeat(1025), false),
            ("é".repeat(1024), true),
            ("é".repeat(1025), false),
        ] {
            let request = EvaluationRequest::new(
                State::text("id limit").unwrap(),
                ModelId::new("clef").unwrap(),
                vec![(
                    QuestionId::new(id).unwrap(),
                    Question::noul(Content::text("ok?").unwrap(), None).unwrap(),
                )],
            )
            .unwrap();
            assert_eq!(
                build_evaluation_request(&Endpoint::huggingface(), &request).is_ok(),
                accepted
            );
        }
    }

    fn sized_png(width: u32, height: u32) -> jev_core::EmbeddedImage {
        let mut bytes = include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec();
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        jev_core::EmbeddedImage::from_bytes(bytes).unwrap()
    }

    #[test]
    fn preflight_size_equals_real_wire_bytes_for_all_media_shapes() {
        let image = sized_png(32, 64);
        for endpoint in [
            Endpoint::ollama(),
            Endpoint::huggingface(),
            cloudflare_endpoint(),
        ] {
            for count in 0..=4 {
                let mut request = EvaluationRequest::new(
                    State::text("escaped \"state\"\n\t\\ Unicode é / slash").unwrap(),
                    ModelId::new("clef").unwrap(),
                    request().questions().to_vec(),
                )
                .unwrap()
                .with_images(vec![image.clone(); count])
                .unwrap();
                if endpoint.provider() == "huggingface" {
                    let video = jev_core::EmbeddedVideo::new(vec![image.clone(); 3])
                        .unwrap()
                        .with_metadata(
                            json!({"fps":30,"total_num_frames":3,"frames_indices":[0,1,2]}),
                        )
                        .unwrap();
                    request = request
                        .with_videos(vec![video])
                        .unwrap()
                        .with_media_kwargs(json!({"max_pixels":4096}))
                        .unwrap()
                        .with_max_length(8192)
                        .unwrap()
                        .with_max_state_tokens(1024)
                        .unwrap();
                }
                if endpoint.provider() == "ollama" {
                    request = request.with_keep_alive(json!("5m")).unwrap();
                }
                if endpoint.is_cloudflare() {
                    request = request.with_reject_if_busy(true);
                }
                assert_eq!(
                    preflight_evaluation_request(&endpoint, &request).unwrap(),
                    build_evaluation_request(&endpoint, &request)
                        .unwrap()
                        .body
                        .len()
                );
            }
        }
    }

    #[test]
    fn preflight_size_matches_normalized_image_forms_and_question_json() {
        for bytes in [
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").as_slice(),
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.jpg").as_slice(),
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.webp").as_slice(),
        ] {
            let image = jev_core::EmbeddedImage::from_bytes(bytes.to_vec()).unwrap();
            let images = [
                image.clone(),
                jev_core::EmbeddedImage::from_raw_base64(&image.base64()).unwrap(),
                jev_core::EmbeddedImage::from_data_url(&format!(
                    "data:{};base64,{}",
                    image.content_type(),
                    image.base64()
                ))
                .unwrap(),
            ];
            for image in images {
                let request = EvaluationRequest::new(
                    State::new(Content::try_from(json!({"escaped":"é\\\"\n/"})).unwrap()),
                    ModelId::new("clef").unwrap(),
                    vec![(
                        QuestionId::new("q").unwrap(),
                        Question::noul(
                            Content::try_from(json!({"question":"é\\\"\n/","number":0.125}))
                                .unwrap(),
                            None,
                        )
                        .unwrap(),
                    )],
                )
                .unwrap()
                .with_images(vec![image.clone()])
                .unwrap();
                for endpoint in [
                    Endpoint::ollama(),
                    Endpoint::huggingface(),
                    cloudflare_endpoint(),
                ] {
                    assert_eq!(
                        preflight_evaluation_request(&endpoint, &request).unwrap(),
                        build_evaluation_request(&endpoint, &request)
                            .unwrap()
                            .body
                            .len()
                    );
                }
                let request = request
                    .with_videos(vec![
                        jev_core::EmbeddedVideo::new(vec![image.clone(), image]).unwrap(),
                    ])
                    .unwrap();
                assert_eq!(
                    preflight_evaluation_request(&Endpoint::huggingface(), &request).unwrap(),
                    build_evaluation_request(&Endpoint::huggingface(), &request)
                        .unwrap()
                        .body
                        .len()
                );
            }
        }
    }
    #[test]
    fn preflight_body_caps_match_exact_media_wire_size_at_the_boundary() {
        let mut bytes = include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec();
        bytes.resize(4 * 1024 * 1024, 0);
        let image = jev_core::EmbeddedImage::from_bytes(bytes).unwrap();
        for endpoint in [
            cloudflare_endpoint(),
            Endpoint::huggingface(),
            Endpoint::ollama(),
        ] {
            let make = |state_bytes| {
                EvaluationRequest::new(
                    State::text("s".repeat(state_bytes)).unwrap(),
                    ModelId::new("clef").unwrap(),
                    request().questions().to_vec(),
                )
                .unwrap()
                .with_images(vec![image.clone()])
                .unwrap()
            };
            let empty = make(1);
            let overhead = preflight_evaluation_request(&endpoint, &empty).unwrap() - 1;
            let cap = request_body_limit(endpoint.provider(), true).unwrap();
            let at_cap = make(cap - overhead);
            assert_eq!(
                preflight_evaluation_request(&endpoint, &at_cap).unwrap(),
                cap
            );
            assert_eq!(
                build_evaluation_request(&endpoint, &at_cap)
                    .unwrap()
                    .body
                    .len(),
                cap
            );
            let over_cap = make(cap - overhead + 1);
            assert!(preflight_evaluation_request(&endpoint, &over_cap).is_err());
            let transport = MockTransport::new();
            let client = Client::with_clock(&transport, endpoint, FakeClock::new());
            let (result, stats) = client.evaluate(&over_cap, &credential());
            assert!(result.is_err());
            assert_eq!(stats.attempts, 0);
            assert!(transport.observed().is_empty());
        }
    }
    #[test]
    fn allocation_estimate_matches_pinned_processor_quantization_and_padding() {
        for (width, height, count, minimum, maximum, temporal, expected) in [
            (64, 64, 2, 4096, 4096, true, (1024.0, 2048.0)),
            (
                512,
                512,
                32,
                4096,
                25_165_824,
                true,
                (262_144.0, 8_388_608.0),
            ),
            (680, 1105, 26, 26658, 65536, true, (2048.0, 53248.0)),
            (
                2,
                3,
                1,
                16_000_000,
                16_000_000,
                false,
                (16_242_688.0, 16_242_688.0),
            ),
        ] {
            assert_eq!(
                resized_pixels(&sized_png(width, height), count, minimum, maximum, temporal),
                expected
            );
        }
    }
    #[test]
    fn bridge_video_pixels_are_a_whole_clip_budget() {
        let video = jev_core::EmbeddedVideo::new(vec![sized_png(512, 512); 32]).unwrap();
        let request = request()
            .with_videos(vec![video])
            .unwrap()
            .with_media_kwargs(json!({"max_pixels":16_000_000}))
            .unwrap();
        assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_ok());
    }

    #[test]
    fn bridge_patch_rounding_cannot_amplify_output_past_frame_limit() {
        let request = request()
            .with_images(vec![sized_png(2, 3)])
            .unwrap()
            .with_media_kwargs(json!({"min_pixels":16_000_000,"max_pixels":16_000_000}))
            .unwrap();
        assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_err());
    }

    #[test]
    fn animated_webp_is_refused_only_by_the_python_bridge() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&22_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8X");
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(&[2, 0, 0, 0, 1, 0, 0, 2, 0, 0]);
        let image = jev_core::EmbeddedImage::from_bytes(bytes).unwrap();
        let request = EvaluationRequest::new(
            State::text("image").unwrap(),
            ModelId::new("clef").unwrap(),
            request().questions().to_vec(),
        )
        .unwrap()
        .with_images(vec![image])
        .unwrap();
        assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_err());
        assert!(build_evaluation_request(&Endpoint::ollama(), &request).is_ok());
        assert!(build_evaluation_request(&cloudflare_endpoint(), &request).is_ok());
    }
    #[test]
    fn python_sampling_maximum_is_a_whole_clip_limit() {
        let image = jev_core::EmbeddedImage::from_bytes(
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec(),
        )
        .unwrap();
        let video = jev_core::EmbeddedVideo::new(vec![image]).unwrap();
        let request = request()
            .with_videos(vec![video])
            .unwrap()
            .with_media_kwargs(
                json!({"do_sample_frames":true,"num_frames":32,"max_pixels":16_000_000}),
            )
            .unwrap();
        // The old assertion treated max_pixels as a per-frame value. The pinned
        // processor applies it to the complete sampled clip, so this is bounded.
        assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_ok());
    }
    #[test]
    fn python_sparse_source_indices_reject_resampling_before_transport() {
        let image = jev_core::EmbeddedImage::from_bytes(
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec(),
        )
        .unwrap();
        let video = jev_core::EmbeddedVideo::new(vec![image.clone(), image])
            .unwrap()
            .with_metadata(json!({"fps":30,"total_num_frames":90,"frames_indices":[0,60]}))
            .unwrap();
        let request = request()
            .with_videos(vec![video])
            .unwrap()
            .with_media_kwargs(json!({"do_sample_frames":true,"num_frames":2}))
            .unwrap();
        assert!(build_evaluation_request(&Endpoint::huggingface(), &request).is_err());
    }
    #[test]
    fn python_bridge_serializes_frames_and_controls_and_rejects_them_on_other_providers() {
        let image = jev_core::EmbeddedImage::from_bytes(
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec(),
        )
        .unwrap();
        let video = jev_core::EmbeddedVideo::new(vec![image.clone(), image.clone()]).unwrap();
        let request = EvaluationRequest::new(
            State::text("frames").unwrap(),
            ModelId::new("clef").unwrap(),
            vec![(
                QuestionId::new("ok").unwrap(),
                Question::noul(Content::text("ok?").unwrap(), None).unwrap(),
            )],
        )
        .unwrap()
        .with_images(vec![image])
        .unwrap()
        .with_videos(vec![video])
        .unwrap()
        .with_max_length(4096)
        .unwrap()
        .with_media_kwargs(json!({"max_pixels":4096}))
        .unwrap();
        let http = build_evaluation_request(&Endpoint::huggingface(), &request).unwrap();
        assert_eq!(http.url, "http://127.0.0.1:8787/v1/systemone");
        let body: serde_json::Value = serde_json::from_slice(&http.body).unwrap();
        assert_eq!(body["videos"][0]["frames"].as_array().unwrap().len(), 2);
        assert_eq!(body["max_length"], 4096);
        assert_eq!(body["media_kwargs"]["max_pixels"], 4096);
        for endpoint in [
            Endpoint::official(),
            Endpoint::ollama(),
            Endpoint::llama_cpp(),
            Endpoint::cloudflare("0123456789abcdef0123456789abcdef").unwrap(),
        ] {
            assert!(build_evaluation_request(&endpoint, &request).is_err());
        }
    }
    use std::cell::RefCell;
    use std::sync::Mutex;

    use jev_core::{Content, ModelId, Question, QuestionId, State};
    use serde_json::json;

    use super::*;
    use crate::testing::MockTransport;

    /// A clock that never really sleeps; it advances a virtual instant instead.
    #[derive(Debug)]
    struct FakeClock {
        state: Mutex<RefCell<Duration>>,
        base: Instant,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                state: Mutex::new(RefCell::new(Duration::ZERO)),
                base: Instant::now(),
            }
        }

        fn slept(&self) -> Duration {
            let guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard.borrow()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + self.slept()
        }

        fn sleep(&self, duration: Duration) {
            let guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = *guard.borrow();
            *guard.borrow_mut() = current + duration;
        }

        fn jitter_sample(&self) -> f64 {
            0.0
        }
    }

    fn credential() -> Credential {
        Credential::new("sk-canary-client-must-not-appear".to_owned())
    }

    fn request() -> EvaluationRequest {
        EvaluationRequest::new(
            State::text("a ticket").unwrap(),
            ModelId::default(),
            vec![(
                QuestionId::new("urgent").unwrap(),
                Question::noul(Content::text("Urgent?").unwrap(), None).unwrap(),
            )],
        )
        .unwrap()
    }

    fn success_body() -> Vec<u8> {
        json!({
            "model": "jev-1.13.0",
            "answers": {"urgent": {"type": "noul", "noul": 0.92}},
            "usage": {"input_tokens": 312, "output_tokens": 48}
        })
        .to_string()
        .into_bytes()
    }

    fn client(transport: MockTransport) -> Client<MockTransport, FakeClock> {
        Client::with_clock(transport, Endpoint::official(), FakeClock::new())
    }

    #[test]
    fn a_successful_evaluation_decodes() {
        let transport = MockTransport::new().with_response(200, success_body());
        let (result, stats) = client(transport).evaluate(&request(), &credential());
        let response = result.unwrap();
        assert_eq!(response.model.as_str(), "jev-1.13.0");
        assert_eq!(response.usage.input_tokens, Some(312));
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.status, Some(200));
    }

    fn clef_request(model: &str) -> EvaluationRequest {
        EvaluationRequest::new(
            request().state().clone(),
            ModelId::new(model).unwrap(),
            request().questions().to_vec(),
        )
        .unwrap()
    }

    fn cloudflare_endpoint() -> Endpoint {
        Endpoint::cloudflare("0123456789abcdef0123456789abcdef").unwrap()
    }

    #[test]
    fn cloudflare_routes_and_normalizes_model_and_decodes_envelope() {
        let body = json!({"result":{"model":"clef-flash","answers":{"urgent":{"type":"noul","noul":0.95}},"usage":{"input_tokens":99,"output_tokens":0}},"success":true,"errors":[],"messages":[]}).to_string();
        let transport = MockTransport::new().with_response_headers(
            200,
            BTreeMap::from([(
                "cf-ray".to_owned(),
                "ray-sk-canary-client-must-not-appear".to_owned(),
            )]),
            body,
        );
        let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
        let (response, stats) =
            client.evaluate(&clef_request("@cf/cloudflare/clef-flash"), &credential());
        assert_eq!(response.unwrap().model.as_str(), "clef-flash");
        assert_eq!(stats.request_id.as_deref(), Some("ray-<redacted>"));
        let sent = transport.observed().remove(0);
        assert_eq!(
            sent.url,
            "https://api.cloudflare.com/client/v4/accounts/0123456789abcdef0123456789abcdef/ai/run/@cf/cloudflare/clef-flash"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&sent.body).unwrap()["model"],
            "clef-flash"
        );
    }

    #[test]
    fn cloudflare_invalid_requests_fail_before_transport() {
        let transport = MockTransport::new();
        let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
        for request in [
            request(),
            EvaluationRequest::new(
                State::text("s").unwrap(),
                ModelId::new("clef").unwrap(),
                vec![(
                    QuestionId::new("not valid").unwrap(),
                    request().questions()[0].1.clone(),
                )],
            )
            .unwrap(),
            EvaluationRequest::new(
                State::text("s").unwrap(),
                ModelId::new("clef").unwrap(),
                (0..65)
                    .map(|n| {
                        (
                            QuestionId::new(format!("q{n}")).unwrap(),
                            request().questions()[0].1.clone(),
                        )
                    })
                    .collect(),
            )
            .unwrap(),
        ] {
            let (result, stats) = client.evaluate(&request, &credential());
            assert!(result.is_err());
            assert_eq!(stats.attempts, 0);
        }
        assert!(transport.observed().is_empty());
    }

    #[test]
    fn cloudflare_errors_never_echo_response_text() {
        for status in [400, 401, 403, 404, 408, 413, 429, 500] {
            let transport = MockTransport::new().with_response(status, br#"{"success":false,"errors":[{"code":3006,"message":"private payload sk-canary-client-must-not-appear"}]}"#.to_vec());
            let client = Client::with_clock(transport, cloudflare_endpoint(), FakeClock::new())
                .with_retry(RetryPolicy::none());
            let (result, _) = client.evaluate(&clef_request("clef"), &credential());
            let error = result.unwrap_err();
            assert_eq!(error.status(), Some(status));
            assert!(!error.to_string().contains("private payload"));
            assert!(!error.to_string().contains("TypeSafe"));
        }
    }

    #[test]
    fn local_provider_does_not_inherit_typesafe_trust() {
        for endpoint in [
            Endpoint::ollama(),
            Endpoint::llama_cpp(),
            Endpoint::official().with_ollama(),
            Endpoint::official().with_llama_cpp(),
        ] {
            assert!(!endpoint.is_official());
            assert!(endpoint.is_local_provider());
        }
    }

    #[test]
    fn ollama_enforces_its_text_body_and_option_limits() {
        let huge = EvaluationRequest::new(
            State::text("s".repeat(64 * 1024)).unwrap(),
            ModelId::new("clef").unwrap(),
            request().questions().to_vec(),
        )
        .unwrap();
        assert!(build_evaluation_request(&Endpoint::ollama(), &huge).is_err());
        let many = EvaluationRequest::new(
            State::text("s").unwrap(),
            ModelId::new("clef").unwrap(),
            vec![(
                QuestionId::new("q").unwrap(),
                Question::choice(
                    Content::text("Which?").unwrap(),
                    (0..27)
                        .map(|n| jev_core::ChoiceOption::new(format!("o{n}"), None).unwrap())
                        .collect(),
                )
                .unwrap(),
            )],
        )
        .unwrap();
        assert!(build_evaluation_request(&Endpoint::ollama(), &many).is_err());
        assert!(build_evaluation_request(&Endpoint::llama_cpp(), &many).is_ok());
    }

    #[test]
    fn provider_model_listing_uses_its_documented_route_and_envelope() {
        let transport = MockTransport::new().with_response(
            200,
            br#"{"success":true,"errors":[],"messages":[],"result":[{}]}"#.to_vec(),
        );
        let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
        let (cards, _) = client.models(&credential());
        let cards = cards.unwrap();
        assert_eq!(
            cards
                .iter()
                .map(|card| card.name.as_str())
                .collect::<Vec<_>>(),
            vec!["clef", "clef-flash"]
        );
        assert!(cards.iter().all(|card| card.release_date.is_empty()));
        assert_eq!(
            transport.observed()[0].url,
            "https://api.cloudflare.com/client/v4/accounts/0123456789abcdef0123456789abcdef/ai/models/search?search=clef"
        );
        let transport = MockTransport::new().with_response(200,br#"{"models":[{"name":"clef:27b","model":"clef:27b","modified_at":"2026-10-01","size":1,"digest":"example","details":{"family":"qwen"}}]}"#.to_vec());
        let client = Client::with_clock(&transport, Endpoint::ollama(), FakeClock::new());
        let (cards, _) = client.models(&Credential::anonymous());
        assert_eq!(cards.unwrap()[0].name, "clef:27b");
        assert_eq!(
            transport.observed()[0].url,
            "http://127.0.0.1:11434/api/tags"
        );
    }

    #[test]
    fn cloudflare_capacity_rejection_is_not_retried() {
        let transport = MockTransport::new().with_response(429,br#"{"success":false,"errors":[{"code":3040,"message":"Capacity temporarily exceeded, please try again."}],"result":{},"messages":[]}"#.to_vec());
        let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
        let request = clef_request("clef").with_reject_if_busy(true);
        let (error, stats) = client.evaluate(&request, &credential());
        assert!(error.unwrap_err().is_unavailable());
        assert_eq!(stats.attempts, 1);
        assert_eq!(client.clock.slept(), Duration::ZERO);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&transport.observed()[0].body).unwrap()["options"],
            json!({"rejectIfBusy":true})
        );
        assert!(build_evaluation_request(&Endpoint::official(), &request).is_err());
    }

    /// Hosted Clef rejected the documented REST option on 2026-10-04 UTC. A
    /// fallback that omits it would silently allow the capacity queue again.
    #[test]
    fn cloudflare_capacity_option_validation_failure_never_falls_back() {
        for model in ["clef", "clef-flash"] {
            // Synthetic diagnostic text exercises redaction; only status 422/code
            // 5012 were retained from live requests, so its field cause is unobserved.
            let transport = MockTransport::new().with_response(
                422,
                br#"{"success":false,"errors":[{"code":5012,"message":"Request body failed validation","details":{"fieldErrors":{"options":["Extra inputs are not permitted"]}}}],"result":{},"messages":[]}"#.to_vec(),
            );
            let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
            let request = clef_request(model).with_reject_if_busy(true);
            let (result, stats) = client.evaluate(&request, &credential());
            let error = result.unwrap_err();
            assert_eq!(error.status(), Some(422));
            assert!(error.to_string().contains("Cloudflare error code 5012"));
            assert!(!error.to_string().contains("Extra inputs"));
            assert_eq!(stats.attempts, 1);
            assert_eq!(client.clock.slept(), Duration::ZERO);
            let observed = transport.observed();
            assert_eq!(observed.len(), 1);
            let body: serde_json::Value = serde_json::from_slice(&observed[0].body).unwrap();
            assert_eq!(body["options"], json!({"rejectIfBusy":true}));
        }
    }

    #[test]
    fn systemone_request_order_stays_as_supplied() {
        let request = EvaluationRequest::new(
            State::text("s").unwrap(),
            ModelId::new("clef").unwrap(),
            vec![
                (
                    QuestionId::new("z_first").unwrap(),
                    request().questions()[0].1.clone(),
                ),
                (
                    QuestionId::new("a_last").unwrap(),
                    request().questions()[0].1.clone(),
                ),
            ],
        )
        .unwrap();
        for endpoint in [
            Endpoint::official(),
            cloudflare_endpoint(),
            Endpoint::ollama(),
            Endpoint::llama_cpp(),
        ] {
            let body =
                String::from_utf8(build_evaluation_request(&endpoint, &request).unwrap().body)
                    .unwrap();
            assert!(body.find("z_first").unwrap() < body.find("a_last").unwrap());
        }
    }

    #[test]
    fn ollama_keep_alive_is_rejected_by_other_providers() {
        let request = clef_request("clef").with_keep_alive(json!("10m")).unwrap();
        let body = build_evaluation_request(&Endpoint::ollama(), &request)
            .unwrap()
            .body;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["keep_alive"],
            "10m"
        );
        for endpoint in [
            Endpoint::official(),
            cloudflare_endpoint(),
            Endpoint::llama_cpp(),
        ] {
            assert!(build_evaluation_request(&endpoint, &request).is_err());
        }
    }

    #[test]
    fn provider_image_wire_formats_and_local_rejection_are_explicit() {
        let image = jev_core::EmbeddedImage::from_bytes(
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec(),
        )
        .unwrap();
        let request = clef_request("clef").with_images(vec![image]).unwrap();
        let cf = serde_json::from_slice::<serde_json::Value>(
            &build_evaluation_request(&cloudflare_endpoint(), &request)
                .unwrap()
                .body,
        )
        .unwrap();
        let ollama = serde_json::from_slice::<serde_json::Value>(
            &build_evaluation_request(&Endpoint::ollama(), &request)
                .unwrap()
                .body,
        )
        .unwrap();
        assert_eq!(cf["images"][0]["content_type"], "image/png");
        assert_eq!(ollama["images"][0], cf["images"][0]["base64"]);
        assert!(ollama["images"][0].is_string());
        assert!(build_evaluation_request(&Endpoint::official(), &request).is_err());
        assert!(build_evaluation_request(&Endpoint::llama_cpp(), &request).is_err());
    }

    #[test]
    fn private_local_protocol_deployments_use_explicit_custom_credentials() {
        let transport = MockTransport::new().with_response(200, success_body());
        let endpoint = Endpoint::parse("https://private.example.com")
            .unwrap()
            .with_ollama();
        let client = Client::with_clock(&transport, endpoint, FakeClock::new());
        let (result, _) = client.evaluate(&clef_request("clef"), &credential());
        assert!(result.is_ok());
        assert_eq!(transport.credentials_seen(), 1);
    }

    #[test]
    fn cloudflare_enforces_body_size_before_transport() {
        let request = EvaluationRequest::new(
            State::text("s".repeat(13 * 1024 * 1024)).unwrap(),
            ModelId::new("clef").unwrap(),
            request().questions().to_vec(),
        )
        .unwrap();
        let transport = MockTransport::new();
        let client = Client::with_clock(&transport, cloudflare_endpoint(), FakeClock::new());
        let (result, stats) = client.evaluate(&request, &credential());
        assert!(result.is_err());
        assert_eq!(stats.attempts, 0);
        assert!(transport.observed().is_empty());
    }

    #[test]
    fn the_request_matches_the_documented_wire_form() {
        let transport = MockTransport::new().with_response(200, success_body());
        let client = client(transport);
        let _ = client.evaluate(&request(), &credential());
        let observed = client.transport_for_test().observed();
        let sent = observed.first().expect("one request");

        assert_eq!(sent.method, "POST");
        assert_eq!(sent.url, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(
            sent.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert!(
            sent.headers
                .get("user-agent")
                .is_some_and(|agent| agent.starts_with("jev-cli/"))
        );
        let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body,
            json!({
                "state": "a ticket",
                "model": "jev-latest",
                "questions": {"urgent": {"type": "noul", "instructions": "Urgent?"}}
            })
        );
    }

    #[test]
    fn a_credential_quoted_back_in_an_error_body_is_redacted() {
        let body = r#"{"detail":"bad key sk-canary-client-must-not-appear in header"}"#;
        let transport = MockTransport::new().with_response(401, body.as_bytes().to_vec());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-client-must-not-appear"),
            "the echoed key survived: {rendered}"
        );
        assert!(
            rendered.contains("bad key <redacted> in header"),
            "{rendered}"
        );
    }

    #[test]
    fn a_credential_straddling_the_message_limit_leaves_no_prefix() {
        let secret = "sk-canary-client-must-not-appear";
        let padding = "x".repeat(jev_core::limits::MAX_ERROR_BODY_CHARS - 10);
        let body = format!(r#"{{"detail":"{padding} {secret}"}}"#);
        let transport = MockTransport::new().with_response(401, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-cl"),
            "a prefix survived: {rendered}"
        );
    }

    #[test]
    fn a_json_escaped_credential_straddling_the_limit_leaves_no_prefix() {
        // `\u0073` is `s`: the raw body does not contain the key, the decoded one does.
        let padding = "x".repeat(jev_core::limits::MAX_ERROR_BODY_CHARS - 10);
        let body = format!(r#"{{"detail":"{padding} \u0073k-canary-client-must-not-appear"}}"#);
        let transport = MockTransport::new().with_response(401, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-cl"),
            "a prefix survived: {rendered}"
        );
    }

    #[test]
    fn a_credential_echoed_in_the_request_id_header_is_redacted() {
        let headers = std::iter::once((
            "x-typesafe-request-id".to_owned(),
            "req sk-canary-client-must-not-appear".to_owned(),
        ))
        .collect();
        let transport = MockTransport::new().with_response_headers(200, headers, success_body());
        let (_, stats) = client(transport).evaluate(&request(), &credential());
        assert_eq!(stats.request_id.as_deref(), Some("req <redacted>"));
    }

    #[test]
    fn the_credential_never_enters_the_request_struct() {
        // The T2 invariant, asserted end to end rather than only at the type level.
        let transport = MockTransport::new().with_response(200, success_body());
        let client = client(transport);
        let _ = client.evaluate(&request(), &credential());
        let observed = client.transport_for_test().observed();
        let rendered = format!("{observed:?}");
        assert!(
            !rendered.contains("sk-canary-client-must-not-appear"),
            "credential reached the recorded request: {rendered}"
        );
        for request in &observed {
            for (name, value) in &request.headers {
                assert!(!value.contains("sk-canary"), "credential in header {name}");
            }
        }
    }

    #[test]
    fn models_uses_a_get_against_the_documented_path() {
        let body = json!({
            "models": [
                {"name": "jev-latest", "description": "flagship", "release_date": "2026-09-15"}
            ]
        })
        .to_string();
        let transport = MockTransport::new().with_response(200, body.into_bytes());
        let client = client(transport);
        let (models, _) = client.models(&credential());
        let models = models.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "jev-latest");

        let observed = client.transport_for_test().observed();
        assert_eq!(observed[0].method, "GET");
        assert_eq!(observed[0].url, "https://api.typesafe.ai/v1/models");
        assert!(observed[0].body.is_empty());
        // No body means no Content-Type: sending one on a GET is a small but real
        // correctness wart that some proxies reject.
        assert!(!observed[0].headers.contains_key("content-type"));
    }

    #[test]
    fn a_429_is_retried_and_then_succeeds() {
        let transport = MockTransport::new()
            .with_response_headers(
                429,
                [("retry-after-ms".to_owned(), "10".to_owned())].into(),
                Vec::new(),
            )
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.is_ok());
        assert_eq!(stats.attempts, 2);
        assert_eq!(client.clock_for_test().slept(), Duration::from_millis(10));
    }

    #[test]
    fn a_401_is_not_retried() {
        let transport = MockTransport::new()
            .with_response(401, br#"{"detail":"Invalid API key"}"#.to_vec())
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        let error = result.unwrap_err();
        assert!(error.is_auth());
        assert_eq!(stats.attempts, 1, "an authentication failure was retried");
        assert!(error.to_string().contains("Invalid API key"));
    }

    #[test]
    fn a_422_reports_the_offending_field() {
        let body = json!({
            "detail": [{"loc": ["body", "questions", "urgency", "criteria"], "msg": "Field required"}]
        })
        .to_string();
        let transport = MockTransport::new().with_response(422, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("questions.urgency.criteria: Field required"),
            "{message}"
        );
    }

    #[test]
    fn retries_are_bounded_and_then_the_error_surfaces() {
        let transport = MockTransport::new()
            .with_response(503, Vec::new())
            .with_response(503, Vec::new())
            .with_response(503, Vec::new())
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.unwrap_err().is_unavailable());
        // Two retries after the first attempt: the queued success is never reached.
        assert_eq!(stats.attempts, 3);
    }

    #[test]
    fn a_connection_failure_is_retried_then_reported() {
        let transport = MockTransport::new()
            .with_failure(TransportError::Unreachable {
                reason: "dns failure".to_owned(),
            })
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.is_ok());
        assert_eq!(stats.attempts, 2);
    }

    #[test]
    fn a_malformed_body_is_a_decode_error_not_a_panic() {
        for body in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            br#"{"model":"jev-latest","answers":{"urgent":{"type":"noul","noul":5}}}"#.to_vec(),
            vec![0xff, 0xfe, 0xfd],
        ] {
            let transport = MockTransport::new().with_response(200, body);
            let (result, _) = client(transport).evaluate(&request(), &credential());
            assert!(matches!(result, Err(ClientError::MalformedResponse { .. })));
        }
    }

    #[test]
    fn dry_run_shows_the_real_body() {
        let client = client(MockTransport::new());
        let built = client.build_evaluation_request(&request()).unwrap();
        assert_eq!(built.url, "https://api.typesafe.ai/v1/systemone");
        let rendered = String::from_utf8(built.body).unwrap();
        assert!(rendered.contains("\"model\":\"jev-latest\""));
        assert!(!rendered.contains("Bearer"));
    }

    #[test]
    fn a_custom_endpoint_is_used_verbatim() {
        let endpoint = Endpoint::parse("http://127.0.0.1:9999/base").unwrap();
        let client = Client::with_clock(
            MockTransport::new().with_response(200, success_body()),
            endpoint,
            FakeClock::new(),
        );
        let _ = client.evaluate(&request(), &credential());
        assert_eq!(
            client.transport_for_test().observed()[0].url,
            "http://127.0.0.1:9999/base/v1/systemone"
        );
    }

    impl<T: Transport, C: Clock> Client<T, C> {
        fn transport_for_test(&self) -> &T {
            &self.transport
        }

        fn clock_for_test(&self) -> &C {
            &self.clock
        }
    }
}
