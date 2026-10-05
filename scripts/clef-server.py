#!/usr/bin/env python3
"""Explicit loopback HTTP bridge for a locally installed publisher Clef release.

This is jev's bridge protocol, not a Cloudflare hosted endpoint. Install dependencies
and download a pinned release yourself; --model-path names that existing directory.
Its publisher-owned joint_schema_model.py is imported and executed only at startup.
Source interface: https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py
No publisher implementation is vendored or reproduced here.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import importlib.util
import io
import json
import math
import os
import struct
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.dont_write_bytecode = True

MAX_BODY = 13 * 1024 * 1024
MAX_FILE = 4 * 1024 * 1024
MAX_MEDIA_BYTES = 8 * 1024 * 1024
MAX_PIXELS = 16_000_000
MAX_TOTAL_PIXELS = 64_000_000
DEFAULT_VIDEO_PIXELS = 25_165_824
MAX_DEPTH = 64
CONTENT_TYPES = {"image/png": "PNG", "image/jpeg": "JPEG", "image/webp": "WEBP"}


class BadRequest(ValueError):
    """An opaque invalid request, never containing input data."""


def require(condition):
    if not condition:
        raise BadRequest("invalid request")


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def bounded_json(raw):
    require(len(raw) <= MAX_BODY)
    depth, quoted, escaped = 0, False, False
    for byte in raw:
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            require(depth <= MAX_DEPTH)
        elif byte in (93, 125):
            depth -= 1
    def reject_constant(_):
        raise BadRequest("invalid JSON")
    def finite_float(value):
        result = float(value)
        require(math.isfinite(result))
        return result
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                          parse_constant=reject_constant, parse_float=finite_float)
    except (UnicodeError, ValueError, RecursionError):
        raise BadRequest("invalid JSON") from None


def dimensions(data, content_type):
    """Inspect only container headers; the decoder later verifies the complete file."""
    if content_type == "image/png":
        require(len(data) >= 24 and data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR")
        return struct.unpack(">II", data[16:24])
    if content_type == "image/webp":
        require(len(data) >= 30 and data[:4] == b"RIFF" and data[8:12] == b"WEBP")
        chunk = data[12:16]
        if chunk == b"VP8X":
            require(not data[20] & 2)  # Animated containers need their own frame budget.
            return (1 + int.from_bytes(data[24:27], "little"),
                    1 + int.from_bytes(data[27:30], "little"))
        if chunk == b"VP8L":
            require(data[20] == 47)
            bits = int.from_bytes(data[21:25], "little")
            return ((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1)
        require(chunk == b"VP8 " and data[23:26] == b"\x9d\x01\x2a")
        width, height = struct.unpack("<HH", data[26:30])
        return width & 0x3FFF, height & 0x3FFF
    require(content_type == "image/jpeg" and data[:2] == b"\xff\xd8")
    position = 2
    while position + 4 <= len(data):
        require(data[position] == 255)
        while position < len(data) and data[position] == 255:
            position += 1
        require(position < len(data))
        marker = data[position]
        position += 1
        if marker in (0xD8, 0x01) or 0xD0 <= marker <= 0xD7:
            continue
        require(marker not in (0xD9, 0xDA) and position + 2 <= len(data))
        size = int.from_bytes(data[position:position + 2], "big")
        require(size >= 2 and position + size <= len(data))
        if marker in (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF):
            require(size >= 7)
            height, width = struct.unpack(">HH", data[position + 3:position + 7])
            return width, height
        position += size
    raise BadRequest("invalid image")


def embedded_image(value):
    require(type(value) is dict and set(value) == {"content_type", "base64"})
    content_type, encoded = value["content_type"], value["base64"]
    require(type(content_type) is str and content_type in CONTENT_TYPES)
    require(type(encoded) is str and len(encoded) <= ((MAX_FILE + 2) // 3) * 4)
    try:
        data = base64.b64decode(encoded, validate=True)
    except (ValueError, binascii.Error):
        raise BadRequest("invalid image") from None
    require(0 < len(data) <= MAX_FILE)
    width, height = dimensions(data, content_type)
    require(width > 0 and height > 0 and width * height <= MAX_PIXELS)
    return data, (width, height), content_type


def decode_image(data, expected_size, expected_type):
    from PIL import Image
    Image.MAX_IMAGE_PIXELS = MAX_PIXELS
    try:
        with Image.open(io.BytesIO(data)) as opened:
            # Multiple container headers can disagree. Trust the allocation budget only
            # when the actual decoder agrees, before load() allocates compressed pixels.
            require(opened.size == expected_size and opened.format == CONTENT_TYPES[expected_type])
            require(getattr(opened, "n_frames", 1) == 1)
            require(opened.width * opened.height <= MAX_PIXELS)
            opened.load()
            converted = opened.convert("RGB")
            # A small PNG can carry highly compressed text. Do not retain that decoded
            # container metadata across all video frames; the processor consumes pixels.
            converted.info.clear()
            return converted
    except Exception:
        # Decoder failures include SyntaxError for malformed PNG chunks. Preserve
        # process-control exceptions, but never expose decoder text or input bytes.
        raise BadRequest("invalid image") from None


def stack_frames(frames):
    import numpy as np
    return np.stack([np.asarray(frame, dtype=np.uint8) for frame in frames], axis=0)


def media_options(value):
    require(type(value) is dict)
    require(set(value) <= {"min_pixels", "max_pixels", "fps", "num_frames", "do_sample_frames"})
    require(not ("fps" in value and "num_frames" in value))
    for key in ("min_pixels", "max_pixels"):
        if key in value:
            require(integer(value[key], 1, MAX_PIXELS))
    require(value.get("min_pixels", 1) <= value.get("max_pixels", MAX_PIXELS))
    if "fps" in value:
        require(type(value["fps"]) in (int, float) and math.isfinite(value["fps"]) and 0 < value["fps"] <= 120)
    if "num_frames" in value:
        require(integer(value["num_frames"], 1, 32))
    if "do_sample_frames" in value:
        require(type(value["do_sample_frames"]) is bool)
    return dict(value)


def resized_pixels(size, count, minimum, maximum, temporal=False):
    """Bound the selected release's 32-pixel spatial and two-frame temporal patches.

    Area limits precede patch quantization, so neither a minimum rounded upwards
    nor odd temporal padding is guaranteed to stay within the nominal maximum.
    This allocation-only calculation runs on checked headers before decoding.
    Source: transformers v5.10.2 Qwen2VL image/Qwen3VL video smart_resize.
    """
    width, height = size
    padded = count + count % 2 if temporal else count
    patches = [round(dimension / 32) for dimension in (width, height)]
    rounded = math.prod(patches) * 1024 * padded
    target = maximum if rounded > maximum else minimum if rounded < minimum else None
    if target is not None:
        area = count * width * height
        if rounded > maximum:
            # Preserve division order: multiplying the reciprocal can round a
            # dimension just below a patch boundary and underestimate allocation.
            divisor = math.sqrt(area / target)
            patches = [max(1, math.floor(dimension / divisor / 32)) for dimension in (width, height)]
        else:
            multiplier = math.sqrt(target / area)
            patches = [math.ceil(dimension * multiplier / 32) for dimension in (width, height)]
    frame_pixels = math.prod(patches) * 1024
    return frame_pixels, padded * frame_pixels


def prepare_request(raw, model_name, decode_image=decode_image, stack_frames=stack_frames):
    request = bounded_json(raw)
    require(type(request) is dict and set(request) <= {
        "model", "state", "questions", "images", "videos", "max_length", "max_state_tokens", "media_kwargs"})
    require(request.get("model") in (model_name, "Cloudflare/" + model_name))
    # Publisher render() accepts every bounded JSON value, including null and blank
    # strings. Presence is separate from value: omitted state is still invalid.
    require("state" in request)
    questions = request.get("questions")
    require(type(questions) is dict and 1 <= len(questions) <= 64)
    for key, question in questions.items():
        require(type(key) is str and key.strip() and len(key) <= 1024 and not any(ord(c) < 32 for c in key))
        require(type(question) is dict and set(question) <= {"type", "instructions", "criteria"})
        kind, criteria = question.get("type"), question.get("criteria")
        require(kind in ("noul", "choice", "score"))
        if kind == "choice":
            require(type(criteria) is dict and 2 <= len(criteria) <= 255 and all(k.strip() for k in criteria))
        elif kind == "score":
            require(type(criteria) is list and 2 <= len(criteria) <= 255)
        elif criteria is not None:
            require(type(criteria) is dict and set(criteria) <= {"true", "false"})
    max_length = request.pop("max_length", 16384)
    require(integer(max_length, 1, 65536))
    if "max_state_tokens" in request:
        require(integer(request["max_state_tokens"], 0, 65536))
    options = media_options(request.get("media_kwargs", {}))
    options.setdefault("do_sample_frames", False)
    images, videos = request.get("images", []), request.get("videos", [])
    require(type(images) is list and len(images) <= 4 and type(videos) is list and len(videos) <= 4)
    require(not (videos and options["do_sample_frames"] and options.get("num_frames") == 1))
    checked_images = [embedded_image(image) for image in images]
    checked_videos = []
    metadata = []
    for video in videos:
        require(type(video) is dict and "frames" in video and set(video) <= {"frames", "metadata"})
        require(type(video["frames"]) is list and 1 <= len(video["frames"]) <= 32)
        frames = [embedded_image(frame) for frame in video["frames"]]
        require(all(size == frames[0][1] for _, size, _ in frames))
        checked_videos.append(frames)
        value = video.get("metadata")
        if value is not None:
            require(type(value) is dict and set(value) <= {"fps", "duration", "total_num_frames", "frames_indices"})
            require(type(value.get("fps")) in (int, float) and math.isfinite(value["fps"]) and 0 < value["fps"] <= 120)
            value = dict(value)
            value.setdefault("total_num_frames", len(frames))
            value.setdefault("frames_indices", list(range(len(frames))))
            require(integer(value["total_num_frames"], len(frames), 10_000_000))
            require(value["total_num_frames"] / value["fps"] <= 86400)
            indices = value["frames_indices"]
            require(type(indices) is list and len(indices) == len(frames))
            require(all(integer(index, 0, value["total_num_frames"] - 1) for index in indices))
            require(all(left < right for left, right in zip(indices, indices[1:])))
            if "duration" in value:
                require(type(value["duration"]) in (int, float) and math.isfinite(value["duration"]) and 0 < value["duration"] <= 86400)
            # Resampling an already sparse sequence loses source indices in the processor.
            require(not options["do_sample_frames"] or
                    (value["total_num_frames"] == len(frames) and indices == list(range(len(frames)))))
            if len(frames) == 1:
                value["frames_indices"] = indices * 2
                value["total_num_frames"] = max(2, value["total_num_frames"])
        metadata.append(value)
    all_media = checked_images + [frame for video in checked_videos for frame in video]
    require(sum(len(data) for data, _, _ in all_media) <= MAX_MEDIA_BYTES)
    require(sum(width * height for _, (width, height), _ in all_media) <= MAX_TOTAL_PIXELS)
    singleton_pixels = sum(video[0][1][0] * video[0][1][1] for video in checked_videos if len(video) == 1)
    require(sum(width * height for _, (width, height), _ in all_media) + singleton_pixels <= MAX_TOTAL_PIXELS)
    processor_counts = [max(len(video), options.get("num_frames", 32))
                        if options["do_sample_frames"] else len(video) for video in checked_videos]
    # Image size limits apply per image; video size limits apply to the entire
    # clip, including temporal padding. A frame count must not divide clip size.
    item_budget = MAX_TOTAL_PIXELS // max(1, len(images) + len(videos))
    require(options.get("max_pixels", 1) <= item_budget)
    require(options.get("min_pixels", 1) <= item_budget)
    image_maximum = options.get("max_pixels", min(MAX_PIXELS, item_budget))
    video_maximum = options.get("max_pixels", min(DEFAULT_VIDEO_PIXELS, item_budget))
    # Both publisher releases use different image/video minimums. Preserve each
    # within our bounded maximum; an explicit minimum applies to both processors.
    image_minimum = options.get("min_pixels", min(65536, image_maximum))
    video_minimum = options.get("min_pixels", min(4096, video_maximum))
    resized_total = 0
    for _, size, _ in checked_images:
        frame_pixels, total_pixels = resized_pixels(size, 1, image_minimum, image_maximum)
        require(frame_pixels <= MAX_PIXELS)
        resized_total += total_pixels
    for video, count in zip(checked_videos, processor_counts):
        # Sampling may change the frame count. Check every possible count without
        # trusting source cadence or duplicating the processor's sampling policy.
        counts = range(2, max(2, count) + 1) if options["do_sample_frames"] else [max(2, count)]
        budgets = [resized_pixels(video[0][1], frames, video_minimum, video_maximum, temporal=True)
                   for frames in counts]
        require(all(frame_pixels <= MAX_PIXELS for frame_pixels, _ in budgets))
        resized_total += max(total_pixels for _, total_pixels in budgets)
    require(resized_total <= MAX_TOTAL_PIXELS)
    # The current video processor uses size, not the image-only legacy max_pixels
    # keyword. Route our five constrained controls to their documented processors.
    request["media_kwargs"] = {
        "images_kwargs": {"size": {"shortest_edge": image_minimum, "longest_edge": image_maximum}},
        "videos_kwargs": {"size": {"shortest_edge": video_minimum, "longest_edge": video_maximum},
                          **{key: value for key, value in options.items()
                             if key in ("fps", "num_frames", "do_sample_frames")}},
    }
    if "num_frames" in options:
        # The installed processor otherwise merges its default fps=2, conflicting
        # with an explicitly requested frame count.
        request["media_kwargs"]["videos_kwargs"]["fps"] = None
    if any(value is not None for value in metadata):
        # Preserve the processor's existing fallback for videos without explicit timing.
        request["media_kwargs"]["videos_kwargs"]["video_metadata"] = [
            value if value is not None else {"fps": 24, "total_num_frames": max(2, len(frames)),
                "frames_indices": list(range(len(frames))) if len(frames) > 1 else [0, 0]}
            for value, frames in zip(metadata, checked_videos)]
    # Validate every header before decoding even the first image or allocating arrays.
    try:
        if images:
            request["images"] = [decode_image(data, size, kind) for data, size, kind in checked_images]
        if videos:
            decoded_videos = []
            for video in checked_videos:
                frames = [decode_image(data, size, kind) for data, size, kind in video]
                if len(frames) == 1:
                    # Qwen's resize requires at least one temporal patch (two frames).
                    frames.append(frames[0])
                decoded_videos.append(stack_frames(frames))
            request["videos"] = decoded_videos
    except Exception:
        raise BadRequest("invalid media") from None
    return request, max_length


def make_server(host, port, model_name, infer):
    require(host == "127.0.0.1" and model_name in ("clef", "clef-flash"))
    class QuietHTTPServer(HTTPServer):
        def handle_error(self, request, client_address):
            # Socket resets can precede handler dispatch, during request headers.
            # The default socketserver diagnostic includes a traceback; request
            # failures must remain opaque even outside our response-writing path.
            pass

    class Handler(BaseHTTPRequestHandler):
        server_version = "jev-clef"
        sys_version = ""

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def log_message(self, *_):
            pass  # HTTP diagnostics must never contain request content or headers.

        def send_json(self, status, value):
            data = json.dumps(value, allow_nan=False, separators=(",", ":")).encode("utf-8")
            if len(data) > MAX_BODY:
                status, data = 500, b'{"error":"inference failed"}'
            self.close_connection = True
            try:
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)
            except OSError:
                pass  # A disconnected client cannot receive another error response.

        def send_error(self, code, message=None, explain=None):
            self.send_json(code, {"error": "request rejected"})

        def permitted(self):
            host_header = self.headers.get("Host", "").lower()
            if len(self.headers.get_all("Host", [])) != 1 or self.headers.get("Origin") is not None or host_header not in (
                "127.0.0.1", "127.0.0.1:" + str(self.server.server_port),
                "localhost", "localhost:" + str(self.server.server_port)):
                self.send_error(403)
                return False
            return True

        def do_GET(self):
            if not self.permitted():
                return
            if self.path != "/v1/models":
                self.send_error(404)
                return
            self.send_json(200, {"models": [{"name": model_name,
                "description": "Loaded local Clef release", "release_date": ""}]})

        def do_POST(self):
            if not self.permitted():
                return
            if self.path != "/v1/systemone":
                self.send_error(404)
                return
            if self.headers.get("Content-Type", "").split(";", 1)[0].strip() != "application/json":
                self.send_error(415)
                return
            lengths = self.headers.get_all("Content-Length", [])
            if len(lengths) != 1 or self.headers.get("Transfer-Encoding") is not None:
                self.send_error(400)
                return
            try:
                require(lengths[0].isascii() and lengths[0].isdigit())
                length = int(lengths[0])
                require(length >= 0)
                if length > MAX_BODY:
                    self.send_error(413)
                    return
                raw = self.rfile.read(length)
                require(len(raw) == length)
                request, max_length = prepare_request(raw, model_name)
            except (ValueError, OSError):
                self.send_error(400)
                return
            try:
                result = infer(request, max_length)
            except ValueError:
                # The publisher reports schemas exceeding the context budget and
                # invalid processor arguments as ValueError. Never echo its text.
                self.send_error(400)
                return
            except Exception:
                self.send_error(500)
                return
            try:
                self.send_json(200, result)
            except Exception:
                self.send_error(500)

        def unsupported(self):
            self.send_error(405)

        do_PUT = do_DELETE = do_PATCH = do_OPTIONS = do_HEAD = do_CONNECT = do_TRACE = unsupported

    # One handler at a time also serializes model access; no thread can race torch state.
    return QuietHTTPServer((host, port), Handler)


def load_inference(model_path, model_name, device, dtype):
    path = Path(model_path).resolve()
    require(path.is_dir() and (path / "joint_schema_model.py").is_file())
    for key in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_DATASETS_OFFLINE", "HF_HUB_DISABLE_TELEMETRY", "DO_NOT_TRACK"):
        os.environ[key] = "1"
    import torch
    import numpy
    import PIL.Image
    spec = importlib.util.spec_from_file_location("joint_schema_model", path / "joint_schema_model.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # Publisher dataclasses resolve their module here.
    spec.loader.exec_module(module)
    model, processor = module.load_release_model(path, device=device, dtype=getattr(torch, dtype), local_files_only=True)
    def infer(request, max_length):
        if "max_state_tokens" not in request:
            return module.systemone(model, processor, request, max_length=max_length)
        # The publisher's convenience function exposes only max_length. Compose its
        # public encoding/answer APIs to preserve the same probability semantics.
        record = dict(request)
        limit = record.pop("max_state_tokens")
        with torch.inference_mode():
            encoded = module.encode_record(processor.tokenizer, record, max_length=max_length,
                                           max_state_tokens=limit, processor=processor)
            target = next(model.parameters()).device
            batch = module.collate_records([encoded], processor.tokenizer.pad_token_id, target)
            logits = model(batch)[0]
            answers = {}
            for question, scores in zip(encoded.questions, logits):
                probabilities = dict(zip(question.option_ids, scores.float().softmax(-1).tolist()))
                answers[question.question_id] = module.systemone_answer(
                    record["questions"][question.question_id], probabilities)
            return {"model": record["model"], "answers": answers,
                    "usage": {"input_tokens": len(encoded.input_ids), "output_tokens": 0}}
    return infer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-path", required=True, help="existing pinned local release directory; its Python code executes at startup")
    parser.add_argument("--model-name", choices=("clef", "clef-flash"), default="clef")
    parser.add_argument("--host", choices=("127.0.0.1",), default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--device", choices=("cpu", "cuda"), default="cuda")
    parser.add_argument("--dtype", choices=("bfloat16", "float16", "float32"), default="bfloat16")
    args = parser.parse_args()
    try:
        require(1 <= args.port <= 65535)
        infer = load_inference(args.model_path, args.model_name, args.device, args.dtype)
        with make_server(args.host, args.port, args.model_name, infer) as server:
            sys.stderr.write(f"Clef bridge listening at http://127.0.0.1:{args.port}\n")
            server.serve_forever()
    except KeyboardInterrupt:
        return 0
    except Exception:
        sys.stderr.write("Clef bridge failed; check local release files, installed dependencies, and device availability\n")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
