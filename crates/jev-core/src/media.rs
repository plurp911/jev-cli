//! Bounded embedded image metadata for Clef requests.
//!
//! Hosted limits: <https://developers.cloudflare.com/workers-ai/models/clef/>.
//! Header probing does not decode pixels or prove that the compressed image is intact.

use std::fmt;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum encoded image file size after base64 decoding (4 MiB).
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum width times height, in pixels (16 megapixels).
pub const MAX_IMAGE_PIXELS: usize = 16_000_000;
/// Maximum images in one request.
pub const MAX_IMAGES: usize = 4;
/// Maximum total encoded image file bytes in one request (8 MiB).
pub const MAX_TOTAL_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Client limit for explicitly supplied local video sequences.
pub const MAX_VIDEOS: usize = 4;
/// Client limit for explicitly supplied frames in one local video.
pub const MAX_VIDEO_FRAMES: usize = 32;
/// Client ceiling on still-image and video-frame pixels before local decompression.
pub const MAX_TOTAL_MEDIA_PIXELS: usize = 64_000_000;
const MAX_BASE64_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;

/// Reasons embedded image input is unusable. Messages never include input bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaError {
    /// An unsupported MIME type or image signature was supplied.
    #[error("images must be PNG, JPEG, or WebP")]
    UnsupportedFormat,
    /// The claimed MIME type disagrees with the image signature.
    #[error("image content_type does not match its file format")]
    ContentTypeMismatch,
    /// The image header cannot supply valid dimensions.
    #[error("image header is malformed or truncated")]
    InvalidImage,
    /// Base64 was not canonical standard-alphabet encoding.
    #[error("image base64 is invalid")]
    InvalidBase64,
    /// An image string was not an embedded base64 data URL.
    #[error("images must use an embedded base64 data URL; remote URLs are not accepted")]
    InvalidDataUrl,
    /// One image exceeds the file-byte budget.
    #[error("an image must not exceed 4 MiB")]
    ImageTooLarge,
    /// One image exceeds the pixel budget.
    #[error("an image must not exceed 16 megapixels")]
    TooManyPixels,
    /// A request exceeds the number of images allowed.
    #[error("a request accepts at most 4 images")]
    TooManyImages,
    /// The sum of image file sizes exceeds the request budget.
    #[error("images in a request must not exceed 8 MiB in total")]
    TotalImagesTooLarge,
    /// A local video needs at least one explicitly supplied frame.
    #[error("a video must contain at least one frame")]
    EmptyVideo,
    /// A local video exceeds the frame budget.
    #[error("a video accepts at most 32 frames")]
    TooManyVideoFrames,
    /// Frame dimensions differ and cannot form one dense video array.
    #[error("all frames in a video must have identical dimensions")]
    InconsistentVideoDimensions,
    /// A request exceeds the local video sequence budget.
    #[error("a request accepts at most 4 videos")]
    TooManyVideos,
    /// Combined still-image and video-frame dimensions exceed the decoding budget.
    #[error("a request must not exceed 64 million total media pixels")]
    TotalMediaPixelsTooLarge,
    /// Invalid source cadence, duration, or frame indices.
    #[error(
        "video metadata requires fps (0 < fps <= 120), optional total_num_frames (1..10000000), duration (0 < seconds <= 86400), and one increasing frame index per supplied frame"
    )]
    InvalidVideoMetadata,
}

/// An explicitly ordered local video frame sequence, with no filesystem or URL input.
///
/// These client limits apply to the local Python serving bridge. The publisher's
/// Python API receives dense video arrays rather than this HTTP representation:
/// <https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py>.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddedVideo {
    frames: Vec<EmbeddedImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<serde_json::Value>,
}

impl EmbeddedVideo {
    /// Validates one through 32 frames with matching dimensions, preserving order.
    ///
    /// # Errors
    /// Returns [`MediaError`] for empty, excessive, differently sized frames, or
    /// over 64 million total header pixels. No image pixels are decompressed here.
    pub fn new(frames: Vec<EmbeddedImage>) -> Result<Self, MediaError> {
        let first = frames.first().ok_or(MediaError::EmptyVideo)?;
        if frames.len() > MAX_VIDEO_FRAMES {
            return Err(MediaError::TooManyVideoFrames);
        }
        if frames
            .iter()
            .any(|frame| frame.width() != first.width() || frame.height() != first.height())
        {
            return Err(MediaError::InconsistentVideoDimensions);
        }
        if frames.iter().map(EmbeddedImage::pixel_len).sum::<usize>() > MAX_TOTAL_MEDIA_PIXELS {
            return Err(MediaError::TotalMediaPixelsTooLarge);
        }
        Ok(Self {
            frames,
            metadata: None,
        })
    }

    /// Attaches bounded source timing. Processor timestamps derive from frame index / fps.
    ///
    /// # Errors
    /// Returns [`MediaError::InvalidVideoMetadata`] for unsupported or inconsistent fields.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Result<Self, MediaError> {
        let valid = metadata.as_object().is_some_and(|object| {
            let fps = object.get("fps").and_then(serde_json::Value::as_f64);
            let total = object
                .get("total_num_frames")
                .map_or(Some(self.frames.len() as u64), serde_json::Value::as_u64);
            let duration = object
                .get("duration")
                .map_or(Some(1.0), serde_json::Value::as_f64);
            let indices = object.get("frames_indices").map(|value| value.as_array());
            object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "fps" | "total_num_frames" | "duration" | "frames_indices"
                )
            }) && fps.is_some_and(|value| value.is_finite() && value > 0.0 && value <= 120.0)
                && duration
                    .is_some_and(|value| value.is_finite() && value > 0.0 && value <= 86400.0)
                && total
                    .is_some_and(|value| value >= self.frames.len() as u64 && value <= 10_000_000)
                && total.zip(fps).is_some_and(|(total, fps)| {
                    u32::try_from(total).is_ok_and(|total| f64::from(total) / fps <= 86400.0)
                })
                && indices.is_none_or(|indices| {
                    indices.is_some_and(|indices| {
                        indices.len() == self.frames.len()
                            && indices.iter().all(|index| {
                                index
                                    .as_u64()
                                    .is_some_and(|index| total.is_some_and(|total| index < total))
                            })
                            && indices.windows(2).all(|pair| {
                                pair.first().and_then(serde_json::Value::as_u64)
                                    < pair.get(1).and_then(serde_json::Value::as_u64)
                            })
                    })
                })
        });
        if !valid {
            return Err(MediaError::InvalidVideoMetadata);
        }
        self.metadata = Some(metadata);
        Ok(self)
    }

    /// Explicit source timing, independent of target sampling fps.
    #[must_use]
    pub const fn metadata(&self) -> Option<&serde_json::Value> {
        self.metadata.as_ref()
    }

    /// The explicitly supplied frames, in temporal order.
    #[must_use]
    pub fn frames(&self) -> &[EmbeddedImage] {
        &self.frames
    }

    /// Total compressed image file bytes for all frames.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.frames.iter().map(EmbeddedImage::byte_len).sum()
    }

    /// Total frame pixels before decompression, bounded by the client video ceiling.
    #[must_use]
    pub fn pixel_len(&self) -> usize {
        self.frames.iter().map(EmbeddedImage::pixel_len).sum()
    }
}

impl fmt::Debug for EmbeddedVideo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddedVideo")
            .field("frames", &self.frames.len())
            .field("bytes", &self.byte_len())
            .field(
                "dimensions",
                &self
                    .frames
                    .first()
                    .map(|frame| (frame.width(), frame.height())),
            )
            .finish_non_exhaustive()
    }
}

impl<'de> Deserialize<'de> for EmbeddedVideo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireVideo {
            frames: Vec<EmbeddedImage>,
            #[serde(default)]
            metadata: Option<MetadataFields>,
        }
        let wire = WireVideo::deserialize(deserializer)?;
        let video = Self::new(wire.frames).map_err(serde::de::Error::custom)?;
        if let Some(metadata) = wire.metadata {
            video
                .with_metadata(metadata.0)
                .map_err(serde::de::Error::custom)
        } else {
            Ok(video)
        }
    }
}

struct MetadataFields(serde_json::Value);

impl<'de> Deserialize<'de> for MetadataFields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldsVisitor;
        impl<'de> Visitor<'de> for FieldsVisitor {
            type Value = MetadataFields;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a video source metadata object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !matches!(
                        key.as_str(),
                        "fps" | "total_num_frames" | "duration" | "frames_indices"
                    ) {
                        return Err(serde::de::Error::custom("unknown video metadata field"));
                    }
                    if fields.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate video metadata field"));
                    }
                    fields.insert(key, map.next_value()?);
                }
                Ok(MetadataFields(serde_json::Value::Object(fields)))
            }
        }
        deserializer.deserialize_map(FieldsVisitor)
    }
}

/// An image with validated format, encoded file size, and header dimensions.
///
/// The same conservative media bounds apply to local adapters; those are client
/// security bounds, not a claim about every local server's supported maximum.
#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddedImage {
    content_type: &'static str,
    // Batch templates share immutable media so row count cannot multiply file bytes.
    bytes: Arc<[u8]>,
    width: usize,
    height: usize,
}

impl EmbeddedImage {
    /// Validates explicitly supplied file bytes without decoding pixels.
    ///
    /// # Errors
    /// Returns [`MediaError`] for unsupported, malformed, or excessive input.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, MediaError> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(MediaError::ImageTooLarge);
        }
        let content_type = match imagesize::image_type(&bytes) {
            Ok(imagesize::ImageType::Png) => {
                // imagesize intentionally probes only the first four signature
                // bytes. Require the complete PNG signature and IHDR header here.
                if !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    || bytes.get(8..16) != Some(b"\0\0\0\rIHDR".as_slice())
                    || bytes.len() < 33
                {
                    return Err(MediaError::InvalidImage);
                }
                "image/png"
            }
            Ok(imagesize::ImageType::Jpeg) => "image/jpeg",
            Ok(imagesize::ImageType::Webp) => "image/webp",
            _ => return Err(MediaError::UnsupportedFormat),
        };
        let size = imagesize::blob_size(&bytes).map_err(|_| MediaError::InvalidImage)?;
        if size.width == 0 || size.height == 0 {
            return Err(MediaError::InvalidImage);
        }
        if size
            .width
            .checked_mul(size.height)
            .is_none_or(|pixels| pixels > MAX_IMAGE_PIXELS)
        {
            return Err(MediaError::TooManyPixels);
        }
        Ok(Self {
            content_type,
            bytes: bytes.into(),
            width: size.width,
            height: size.height,
        })
    }

    /// Decodes bounded base64 and checks its declared MIME type against the header.
    ///
    /// # Errors
    /// Returns [`MediaError`] for malformed encoding, mismatched MIME, or limits.
    pub fn from_base64(content_type: &str, encoded: &str) -> Result<Self, MediaError> {
        if !matches!(content_type, "image/png" | "image/jpeg" | "image/webp") {
            return Err(MediaError::UnsupportedFormat);
        }
        let image = Self::from_raw_base64(encoded)?;
        if image.content_type != content_type {
            return Err(MediaError::ContentTypeMismatch);
        }
        Ok(image)
    }

    /// Decodes native Ollama base64 image input and infers MIME from its signature.
    ///
    /// This constructor is explicit: generic image JSON deserialization continues
    /// to require Cloudflare's data URL or MIME/base64 object shape.
    ///
    /// # Errors
    /// Returns [`MediaError`] for malformed encoding, headers, or excessive input.
    pub fn from_raw_base64(encoded: &str) -> Result<Self, MediaError> {
        if encoded.len() > MAX_BASE64_BYTES {
            return Err(MediaError::ImageTooLarge);
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| MediaError::InvalidBase64)?;
        Self::from_bytes(bytes)
    }

    /// Parses a PNG/JPEG/WebP base64 data URL. Remote URLs are never fetched.
    ///
    /// # Errors
    /// Returns [`MediaError`] for unsupported data URLs, encoding, or image limits.
    pub fn from_data_url(value: &str) -> Result<Self, MediaError> {
        let Some(prefix) = value.get(..5) else {
            return Err(MediaError::InvalidDataUrl);
        };
        if !prefix.eq_ignore_ascii_case("data:") {
            return Err(MediaError::InvalidDataUrl);
        }
        let (header, encoded) = value
            .get(5..)
            .and_then(|value| value.split_once(','))
            .ok_or(MediaError::InvalidDataUrl)?;
        let (content_type, encoding) = header.split_once(';').ok_or(MediaError::InvalidDataUrl)?;
        if !encoding.eq_ignore_ascii_case("base64") {
            return Err(MediaError::InvalidDataUrl);
        }
        // Data URL media and encoding tokens are ASCII case-insensitive. Keep the
        // typed object MIME enum strict, and do not accept additional parameters.
        let content_type = ["image/png", "image/jpeg", "image/webp"]
            .into_iter()
            .find(|mime| content_type.eq_ignore_ascii_case(mime))
            .ok_or(MediaError::UnsupportedFormat)?;
        Self::from_base64(content_type, encoded)
    }

    /// MIME type inferred from the file signature.
    #[must_use]
    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }
    /// Encoded image file bytes, before base64 encoding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Canonical standard-alphabet base64 of the encoded image file bytes.
    #[must_use]
    pub fn base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }
    /// Number of encoded image file bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
    /// Header width times height, checked and bounded during construction.
    #[must_use]
    pub const fn pixel_len(&self) -> usize {
        self.width * self.height
    }
    /// Width obtained from the image header.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }
    /// Height obtained from the image header.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }
}

impl fmt::Debug for EmbeddedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddedImage")
            .field("content_type", &self.content_type)
            .field("byte_len", &self.byte_len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Serialize for EmbeddedImage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_struct("EmbeddedImage", 2)?;
        object.serialize_field("content_type", self.content_type)?;
        object.serialize_field("base64", &self.base64())?;
        object.end()
    }
}

impl<'de> Deserialize<'de> for EmbeddedImage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ImageVisitor;
        impl<'de> Visitor<'de> for ImageVisitor {
            type Value = EmbeddedImage;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an embedded image data URL or content_type/base64 object")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                EmbeddedImage::from_data_url(value).map_err(E::custom)
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut content_type: Option<String> = None;
                let mut encoded: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "content_type" if content_type.is_none() => {
                            content_type = Some(map.next_value()?);
                        }
                        "base64" if encoded.is_none() => encoded = Some(map.next_value()?),
                        "content_type" | "base64" => {
                            return Err(serde::de::Error::custom("duplicate embedded image field"));
                        }
                        _ => return Err(serde::de::Error::custom("unknown embedded image field")),
                    }
                }
                let content_type =
                    content_type.ok_or_else(|| serde::de::Error::missing_field("content_type"))?;
                let encoded = encoded.ok_or_else(|| serde::de::Error::missing_field("base64"))?;
                EmbeddedImage::from_base64(&content_type, &encoded)
                    .map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_any(ImageVisitor)
    }
}
