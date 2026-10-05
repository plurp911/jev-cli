//! Explicit media and provider options shared by CLI requests and MCP calls.

use crate::context::Context;
use crate::errors::{CliError, Result};
use jev_client::Endpoint;
use jev_core::{EmbeddedImage, EmbeddedVideo, EvaluationRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read as _;

/// Invocation data that accompanies state and questions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Features {
    #[serde(default)]
    pub(crate) images: Vec<EmbeddedImage>,
    #[serde(default)]
    pub(crate) options: Option<CapacityOptions>,
    #[serde(default)]
    pub(crate) keep_alive: Option<Value>,
    #[serde(default)]
    pub(crate) videos: Vec<EmbeddedVideo>,
    #[serde(default)]
    pub(crate) max_length: Option<u32>,
    /// Independent local state token budget.
    #[serde(default)]
    pub(crate) max_state_tokens: Option<u32>,
    #[serde(default)]
    pub(crate) media_kwargs: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapacityOptions {
    #[serde(rename = "rejectIfBusy")]
    pub(crate) reject_if_busy: bool,
}

impl Features {
    /// Reads only image paths explicitly named on this invocation.
    pub(crate) fn for_cli(context: &Context, mut document: Self) -> Result<Self> {
        Self::check_cli_media_sources(context, &document)?;
        if context.image_paths.len() > 4 || context.video_frames.len() > 32 {
            return Err(CliError::usage(
                "at most 4 images or 32 explicitly named video frames are accepted",
            ));
        }
        let mut total: usize = document
            .images
            .iter()
            .map(EmbeddedImage::byte_len)
            .sum::<usize>()
            + document
                .videos
                .iter()
                .map(EmbeddedVideo::byte_len)
                .sum::<usize>();
        let mut read = |path: &std::path::Path| -> Result<EmbeddedImage> {
            let image = read_image(path)?;
            total = total.saturating_add(image.byte_len());
            if total > 8 * 1024 * 1024 {
                return Err(CliError::usage(
                    "combined images and video frames exceed 8 MiB",
                ));
            }
            Ok(image)
        };
        for path in &context.image_paths {
            document.images.push(read(path)?);
        }
        if !context.video_frames.is_empty() {
            let frames = context
                .video_frames
                .iter()
                .map(|path| read(path))
                .collect::<Result<Vec<_>>>()?;
            let mut video =
                EmbeddedVideo::new(frames).map_err(|error| CliError::usage(error.to_string()))?;
            if let Some(fps) = context.video_fps {
                video = video
                    .with_metadata(serde_json::json!({"fps": fps}))
                    .map_err(|error| CliError::usage(error.to_string()))?;
            }
            document.videos.push(video);
        }
        document.with_context_options(context)
    }

    /// Combines already-loaded CLI media with a document without reading paths twice.
    pub(crate) fn merge_cli(&self, context: &Context, document: Self) -> Result<Self> {
        Self::check_cli_media_sources(context, &document)?;
        self.merge(document)?.with_context_options(context)
    }

    fn check_cli_media_sources(context: &Context, document: &Self) -> Result<()> {
        if !context.image_paths.is_empty() && !document.images.is_empty() {
            return Err(CliError::usage(
                "images were supplied both by --image and the request document; choose one source",
            ));
        }
        if !context.video_frames.is_empty() && !document.videos.is_empty() {
            return Err(CliError::usage(
                "videos were supplied both by --video-frame and the request document; choose one source",
            ));
        }
        Ok(())
    }

    /// CLI flags override options in the request document.
    pub(crate) fn with_context_options(mut self, context: &Context) -> Result<Self> {
        if context.reject_if_busy {
            self.options = Some(CapacityOptions {
                reject_if_busy: true,
            });
        }
        if let Some(value) = &context.keep_alive {
            self.keep_alive = Some(value.parse::<i64>().map_or_else(
                |_| Value::String(value.clone()),
                |seconds| Value::Number(seconds.into()),
            ));
        }
        if context.max_state_tokens.is_some() {
            self.max_state_tokens = context.max_state_tokens;
        }
        if context.max_length.is_some() {
            self.max_length = context.max_length;
        }
        if let Some(kwargs) = &context.media_kwargs {
            self.media_kwargs = Some(kwargs.clone());
        }
        self.validate_provider(context)
    }

    /// Explicit MCP call options override startup defaults. Embedded media remains
    /// supplied by the call; startup arguments cannot discover files for a tool.
    pub(crate) fn with_mcp_defaults(self, context: &Context) -> Result<Self> {
        Self::default()
            .with_context_options(context)?
            .merge(self)?
            .validate_provider(context)
    }

    fn validate_provider(self, context: &Context) -> Result<Self> {
        if (!self.videos.is_empty()
            || self.max_length.is_some()
            || self.max_state_tokens.is_some()
            || self.media_kwargs.is_some())
            && context.endpoint.value.provider() != "huggingface"
        {
            return Err(CliError::usage(
                "videos, max_length, max_state_tokens, and media_kwargs require the Hugging Face Python bridge",
            ));
        }
        if self.options.is_some() && !context.endpoint.value.is_cloudflare() {
            return Err(CliError::usage(
                "capacity options are supported only by Cloudflare",
            ));
        }
        if self.keep_alive.is_some() && context.endpoint.value.provider() != "ollama" {
            return Err(CliError::usage("keep_alive is supported only by Ollama"));
        }
        Ok(self)
    }

    /// Template media and row media are alternatives, avoiding silent duplication.
    pub(crate) fn merge(&self, mut row: Self) -> Result<Self> {
        if !self.images.is_empty() && !row.images.is_empty() {
            return Err(CliError::usage(
                "images were supplied both by the request template and an input row; choose one source",
            ));
        }
        if !self.videos.is_empty() && !row.videos.is_empty() {
            return Err(CliError::usage(
                "videos were supplied both by the request template and an input row; choose one source",
            ));
        }
        if row.videos.is_empty() {
            row.videos.clone_from(&self.videos);
        }
        if row.max_state_tokens.is_none() {
            row.max_state_tokens = self.max_state_tokens;
        }
        if row.max_length.is_none() {
            row.max_length = self.max_length;
        }
        if row.media_kwargs.is_none() {
            row.media_kwargs.clone_from(&self.media_kwargs);
        }
        if row.images.is_empty() {
            row.images.clone_from(&self.images);
        }
        if row.options.is_none() {
            row.options.clone_from(&self.options);
        }
        if row.keep_alive.is_none() {
            row.keep_alive.clone_from(&self.keep_alive);
        }
        Ok(row)
    }

    pub(crate) fn apply(&self, request: EvaluationRequest) -> Result<EvaluationRequest> {
        let mut request = request
            .with_images(self.images.clone())
            .map_err(|error| CliError::usage(error.to_string()))?
            .with_reject_if_busy(
                self.options
                    .as_ref()
                    .is_some_and(|options| options.reject_if_busy),
            );
        request = request
            .with_videos(self.videos.clone())
            .map_err(|error| CliError::usage(error.to_string()))?;
        if let Some(value) = self.max_state_tokens {
            request = request
                .with_max_state_tokens(value)
                .map_err(|error| CliError::usage(error.to_string()))?;
        }
        if let Some(value) = self.max_length {
            request = request
                .with_max_length(value)
                .map_err(|error| CliError::usage(error.to_string()))?;
        }
        if let Some(value) = &self.media_kwargs {
            request = request
                .with_media_kwargs(value.clone())
                .map_err(|error| CliError::usage(error.to_string()))?;
        }
        if let Some(value) = &self.keep_alive {
            request = request
                .with_keep_alive(value.clone())
                .map_err(|error| CliError::usage(error.to_string()))?;
        }
        Ok(request)
    }
}

fn read_image(path: &std::path::Path) -> Result<EmbeddedImage> {
    const IMAGE_BYTES: u64 = 4 * 1024 * 1024;
    let file = open_image(path).map_err(|error| {
        CliError::usage(format!(
            "cannot safely open {} as a regular media file: {}",
            crate::output::Safe::new(&path.display().to_string()),
            error.kind()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take(IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::usage(format!("cannot read the media file: {}", error.kind()))
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > IMAGE_BYTES {
        return Err(CliError::usage(
            "media file exceeds the 4 MiB image byte limit",
        ));
    }
    EmbeddedImage::from_bytes(bytes).map_err(|error| CliError::usage(error.to_string()))
}

/// Provider validation is exercised before a credential is resolved or a socket opened.
pub(crate) fn preflight(endpoint: &Endpoint, request: &EvaluationRequest) -> Result<()> {
    jev_client::preflight_evaluation_request(endpoint, request)
        .map(|_| ())
        .map_err(|error| CliError::usage(error.to_string()))
}

pub(crate) fn score_max(endpoint: &Endpoint) -> usize {
    match endpoint.provider() {
        "huggingface" => jev_core::limits::SCORE_ABSOLUTE_MAX_LEVELS,
        "ollama" => jev_core::limits::OLLAMA_SCORE_MAX_LEVELS,
        _ => jev_core::limits::SCORE_MAX_LEVELS,
    }
}

pub(crate) fn needs_authorization(endpoint: &Endpoint) -> bool {
    !(endpoint.is_local_provider() && endpoint.is_loopback())
}

// Kept pure so the macOS-specific trust decision can be exercised on every test host.
#[cfg(any(target_os = "macos", test))]
fn verified_macos_alias_path(
    path: &std::path::Path,
    observed_target: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    for (alias, target, relative_target) in [
        ("/tmp", "/private/tmp", "private/tmp"),
        ("/var", "/private/var", "private/var"),
        ("/etc", "/private/etc", "private/etc"),
    ] {
        if let Ok(suffix) = path.strip_prefix(alias) {
            if observed_target != std::path::Path::new(target)
                && observed_target != std::path::Path::new(relative_target)
            {
                return Err(std::io::ErrorKind::InvalidInput.into());
            }
            return Ok(std::path::Path::new(target).join(suffix));
        }
    }
    Ok(path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn macos_media_path(
    path: &std::path::Path,
) -> std::io::Result<std::borrow::Cow<'_, std::path::Path>> {
    for alias in ["/tmp", "/var", "/etc"] {
        if path.starts_with(alias) {
            let target = match std::fs::read_link(alias) {
                Ok(target) => target,
                // A real root directory needs no alias exception; the handle walk checks it.
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                    return Ok(std::borrow::Cow::Borrowed(path));
                }
                Err(error) => return Err(error),
            };
            return verified_macos_alias_path(path, &target).map(std::borrow::Cow::Owned);
        }
    }
    Ok(std::borrow::Cow::Borrowed(path))
}

/// Walks directory handles so a concurrent rename cannot redirect an image upload.
#[cfg(unix)]
fn open_image(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    use std::path::Component;
    // macOS owns these root aliases. Only their exact standard targets are trusted;
    // every rewritten component still goes through the same no-follow handle walk.
    #[cfg(target_os = "macos")]
    let path = macos_media_path(path)?;
    let mut components = path.components();
    let last = components
        .next_back()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let mut directory = open(
        if path.is_absolute() { "/" } else { "." },
        flags | OFlags::DIRECTORY,
        Mode::empty(),
    )?;
    for component in components {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                directory = openat(&directory, name, flags | OFlags::DIRECTORY, Mode::empty())?;
            }
            Component::ParentDir => {
                directory = openat(&directory, "..", flags | OFlags::DIRECTORY, Mode::empty())?;
            }
            Component::Prefix(_) => return Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
    let Component::Normal(name) = last else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    let file = std::fs::File::from(openat(&directory, name, flags, Mode::empty())?);
    if !file.metadata()?.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    Ok(file)
}

#[cfg(windows)]
fn open_image(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const REPARSE_POINT: u32 = 0x400;
    const SHARE_READ_WRITE: u32 = 3;
    let mut held = Vec::new();
    let ancestors: Vec<_> = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect();
    // Keeping every opened ancestor denies rename/delete while descendants are opened.
    // OPEN_REPARSE_POINT makes metadata describe the directory itself, not its target.
    for ancestor in ancestors.into_iter().rev() {
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_WRITE)
            .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
            .open(ancestor)?;
        let metadata = directory.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        held.push(directory);
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_image(_path: &std::path::Path) -> std::io::Result<std::fs::File> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Decodes an explicitly selected image field directly from JSON, retaining duplicates.
pub(crate) fn parse_image_field(
    text: &str,
    field: &str,
    origin: &str,
    ollama: bool,
) -> Result<Vec<EmbeddedImage>> {
    use serde::de::{DeserializeSeed, MapAccess, Visitor};
    struct NativeImage(EmbeddedImage);
    impl<'de> Deserialize<'de> for NativeImage {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct NativeVisitor;
            impl<'de> Visitor<'de> for NativeVisitor {
                type Value = NativeImage;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("an image object, data URL, or native base64 string")
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    raw: &str,
                ) -> std::result::Result<Self::Value, E> {
                    let image = if raw
                        .get(..5)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
                    {
                        EmbeddedImage::from_data_url(raw)
                    } else {
                        EmbeddedImage::from_raw_base64(raw)
                    };
                    image.map(NativeImage).map_err(E::custom)
                }
                fn visit_map<M: MapAccess<'de>>(
                    self,
                    access: M,
                ) -> std::result::Result<Self::Value, M::Error> {
                    EmbeddedImage::deserialize(serde::de::value::MapAccessDeserializer::new(access))
                        .map(NativeImage)
                }
            }
            deserializer.deserialize_any(NativeVisitor)
        }
    }
    struct ImageField<'a> {
        field: &'a str,
        ollama: bool,
    }
    impl<'de> Visitor<'de> for ImageField<'_> {
        type Value = Vec<EmbeddedImage>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object containing embedded images")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut access: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut images = None;
            while let Some(key) = access.next_key::<String>()? {
                if key == self.field {
                    if images.is_some() {
                        return Err(serde::de::Error::custom("duplicate image field"));
                    }
                    images = Some(if self.ollama {
                        access
                            .next_value::<Vec<NativeImage>>()?
                            .into_iter()
                            .map(|image| image.0)
                            .collect()
                    } else {
                        access.next_value::<Vec<EmbeddedImage>>()?
                    });
                } else {
                    let _ = access.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(images.unwrap_or_default())
        }
    }
    impl<'de> DeserializeSeed<'de> for ImageField<'_> {
        type Value = Vec<EmbeddedImage>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            deserializer.deserialize_map(self)
        }
    }
    ImageField { field, ollama }
        .deserialize(&mut serde_json::Deserializer::from_str(text))
        .map_err(|error| CliError::usage(format!("{origin}: invalid embedded images: {error}")))
}

/// Parses bridge features directly so duplicate video/frame/options fields are refused.
pub(crate) fn document_features(text: &str, origin: &str, ollama: bool) -> Result<Features> {
    #[derive(Deserialize, Default)]
    struct Extra {
        #[serde(default)]
        videos: Vec<EmbeddedVideo>,
        #[serde(default)]
        options: Option<CapacityOptions>,
        #[serde(default)]
        keep_alive: Option<Value>,
        #[serde(default)]
        max_length: Option<u32>,
        #[serde(default)]
        max_state_tokens: Option<u32>,
        #[serde(
            default,
            deserialize_with = "crate::ordered::deserialize_optional_unambiguous_value"
        )]
        media_kwargs: Option<Value>,
    }
    let extra: Extra = serde_json::from_str(text).map_err(|error| {
        CliError::usage(format!("{origin}: invalid provider features: {error}"))
    })?;
    Ok(Features {
        images: parse_image_field(text, "images", origin, ollama)?,
        videos: extra.videos,
        options: extra.options,
        keep_alive: extra.keep_alive,
        max_length: extra.max_length,
        max_state_tokens: extra.max_state_tokens,
        media_kwargs: extra.media_kwargs,
    })
}

pub(crate) fn parse_video_field(
    text: &str,
    field: &str,
    origin: &str,
) -> Result<Vec<EmbeddedVideo>> {
    use serde::de::{DeserializeSeed, MapAccess, Visitor};
    struct VideoField<'a>(&'a str);
    impl<'de> Visitor<'de> for VideoField<'_> {
        type Value = Vec<EmbeddedVideo>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object containing embedded videos")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut access: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut videos = None;
            while let Some(key) = access.next_key::<String>()? {
                if key == self.0 {
                    if videos.is_some() {
                        return Err(serde::de::Error::custom("duplicate video field"));
                    }
                    videos = Some(access.next_value::<Vec<EmbeddedVideo>>()?);
                } else {
                    let _ = access.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(videos.unwrap_or_default())
        }
    }
    impl<'de> DeserializeSeed<'de> for VideoField<'_> {
        type Value = Vec<EmbeddedVideo>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            d: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            d.deserialize_map(self)
        }
    }
    VideoField(field)
        .deserialize(&mut serde_json::Deserializer::from_str(text))
        .map_err(|error| CliError::usage(format!("{origin}: invalid embedded videos: {error}")))
}

#[cfg(test)]
mod path_tests {
    use std::path::Path;

    #[test]
    fn standard_macos_root_aliases_preserve_the_named_suffix() {
        for (named, observed, expected) in [
            ("/tmp/image.png", "private/tmp", "/private/tmp/image.png"),
            (
                "/var/folders/session/frame.webp",
                "/private/var",
                "/private/var/folders/session/frame.webp",
            ),
            ("/etc/image.jpg", "private/etc", "/private/etc/image.jpg"),
            (
                "/tmp/nested/../image.png",
                "/private/tmp",
                "/private/tmp/nested/../image.png",
            ),
        ] {
            assert!(
                super::verified_macos_alias_path(Path::new(named), Path::new(observed))
                    .is_ok_and(|actual| actual == Path::new(expected))
            );
        }
    }

    #[test]
    fn unexpected_macos_root_alias_targets_are_refused() {
        for (named, observed) in [
            ("/tmp/image.png", "/home/user/tmp"),
            ("/var/image.png", "private/tmp"),
            ("/etc/image.png", "../private/etc"),
            ("/tmp/image.png", "/private/tmp/other"),
        ] {
            assert!(
                super::verified_macos_alias_path(Path::new(named), Path::new(observed)).is_err()
            );
        }
    }

    #[test]
    fn macos_alias_mapping_does_not_rewrite_user_paths_or_similar_names() {
        for named in [
            "tmp/image.png",
            "/temporary/image.png",
            "/private/tmp/image.png",
            "/home/user/tmp/image.png",
        ] {
            assert!(
                super::verified_macos_alias_path(Path::new(named), Path::new("private/tmp"))
                    .is_ok_and(|actual| actual == Path::new(named))
            );
        }
    }
}

#[cfg(test)]
mod native_image_errors {
    use super::parse_image_field;
    #[test]
    fn ollama_image_errors_preserve_declared_content_type_mismatch() {
        let encoded = jev_core::EmbeddedImage::from_bytes(
            include_bytes!("../../jev-core/tests/fixtures/two-by-three.png").to_vec(),
        )
        .unwrap()
        .base64();
        let input = serde_json::json!({"images":[{"content_type":"image/jpeg","base64":encoded}]})
            .to_string();
        let error = parse_image_field(&input, "images", "test", true).unwrap_err();
        assert!(error.to_string().contains("does not match"), "{error}");
    }
    #[test]
    fn ollama_data_url_errors_preserve_size_limit() {
        let input = serde_json::json!({"images":[format!("data:image/png;base64,{}", "A".repeat(6_000_000))]}).to_string();
        let error = parse_image_field(&input, "images", "test", true).unwrap_err();
        assert!(error.to_string().contains("4 MiB"), "{error}");
    }
}
