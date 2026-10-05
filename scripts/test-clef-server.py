#!/usr/bin/env python3
"""Exercise the local Clef bridge without importing torch or downloading weights."""

from __future__ import annotations

import base64
import contextlib
import http.client
import importlib.util
import json
import subprocess
import os
import sys
import struct
import socket
import tempfile
import threading
import types
import zlib
import unittest
from unittest.mock import patch
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "clef-server.py"
PNG = (ROOT / "crates/jev-core/tests/fixtures/two-by-three.png").read_bytes()


def body(**extra):
    return {"model": "clef", "state": "invoice", "questions": {
        "paid": {"type": "noul", "instructions": "Was it paid?"}}, **extra}


def image(data=PNG):
    return {"content_type": "image/png", "base64": base64.b64encode(data).decode("ascii")}


class BridgeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bridge = None
        if SCRIPT.is_file():
            spec = importlib.util.spec_from_file_location("clef_server", SCRIPT)
            cls.bridge = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.bridge)

    def setUp(self):
        self.assertIsNotNone(self.bridge, "the requested local serving bridge does not exist")

    def parse(self, value):
        return self.bridge.prepare_request(json.dumps(value).encode(), "clef",
            decode_image=lambda data, size, kind: ("rgb", data), stack_frames=lambda frames: tuple(frames))

    def test_media_is_decoded_and_frame_order_and_controls_reach_inference(self):
        request, length = self.parse(body(images=[image()], videos=[{"frames": [image(), image()]}],
            max_length=4096, media_kwargs={"max_pixels": 4096, "do_sample_frames": False}))
        self.assertEqual(length, 4096)
        self.assertEqual(request["images"], [("rgb", PNG)])
        self.assertEqual(request["videos"], [(("rgb", PNG), ("rgb", PNG))])
        self.assertEqual(request["media_kwargs"]["images_kwargs"]["size"]["longest_edge"], 4096)
        self.assertFalse(request["media_kwargs"]["videos_kwargs"]["do_sample_frames"])
        self.assertNotIn("max_length", request)

    def test_explicit_video_source_timing_reaches_processor_and_state_budget_is_independent(self):
        request, length = self.parse(body(max_length=4096, max_state_tokens=128,
            videos=[{"frames": [image(), image()], "metadata": {
                "fps": 30, "total_num_frames": 90, "frames_indices": [0, 60], "duration": 3}}]))
        self.assertEqual(length, 4096)
        self.assertEqual(request["max_state_tokens"], 128)
        self.assertEqual(request["media_kwargs"]["videos_kwargs"]["video_metadata"], [
            {"fps": 30, "total_num_frames": 90, "frames_indices": [0, 60], "duration": 3}])

    def test_metadata_and_state_budget_reject_unknown_nonfinite_and_misaligned_input(self):
        for metadata in ({"fps": 0}, {"fps": True}, {"fps": 121}, {"fps": 2, "url": "x"},
                         {"fps": 2, "frames_indices": [0]}, {"fps": 2, "frames_indices": [1, 0]},
                         {"fps": 2, "total_num_frames": 1}, {"fps": 2, "duration": 0},
                         {"fps": 2, "total_num_frames": 8, "frames_indices": [0, 8]}):
            with self.subTest(metadata=metadata), self.assertRaises(self.bridge.BadRequest):
                self.parse(body(videos=[{"frames": [image(), image()], "metadata": metadata}]))
        for value in (-1, 65537, True, 1.5):
            with self.subTest(value=value), self.assertRaises(self.bridge.BadRequest):
                self.parse(body(max_state_tokens=value))

    def test_explicit_frame_count_disables_processor_default_target_fps(self):
        request, _ = self.parse(body(videos=[{"frames": [image(), image()]}],
                                    media_kwargs={"num_frames": 2, "do_sample_frames": True}))
        self.assertIn("fps", request["media_kwargs"]["videos_kwargs"])
        self.assertIsNone(request["media_kwargs"]["videos_kwargs"]["fps"])

    def test_sparse_source_indices_cannot_be_resampled_or_have_unbounded_time(self):
        for metadata, controls in (({"fps":30,"total_num_frames":90,"frames_indices":[0,60]}, {"do_sample_frames": True}),
                                   ({"fps":1e-12}, {}), ({"fps":1,"total_num_frames":10000000}, {})):
            with self.subTest(metadata=metadata), self.assertRaises(self.bridge.BadRequest):
                self.parse(body(videos=[{"frames": [image(), image()], "metadata": metadata}], media_kwargs=controls))

    def test_processor_minimums_preserve_release_defaults_and_explicit_override(self):
        for controls, image_minimum, video_minimum in (
            ({}, 65536, 4096),
            ({"max_pixels": 1024}, 1024, 1024),
            ({"min_pixels": 8192}, 8192, 8192),
        ):
            with self.subTest(controls=controls):
                request, _ = self.parse(body(images=[image()], videos=[{"frames": [image(), image()]}],
                                            media_kwargs=controls))
                kwargs = request["media_kwargs"]
                self.assertEqual(kwargs["images_kwargs"]["size"]["shortest_edge"], image_minimum)
                self.assertEqual(kwargs["videos_kwargs"]["size"]["shortest_edge"], video_minimum)

    def test_malformed_deep_duplicate_and_nonfinite_json_are_rejected(self):
        for raw in (b"{", b"\xff", b'{"model":"clef","model":"clef"}',
                    b"[" * 65 + b"0" + b"]" * 65, b'{"state":NaN}', b'{"state":1e9999}'):
            with self.subTest(raw=raw[:20]), self.assertRaises(self.bridge.BadRequest):
                self.bridge.prepare_request(raw, "clef")

    def test_no_paths_urls_or_arbitrary_processor_or_generation_arguments(self):
        for extra in ({"images": ["/etc/passwd"]}, {"videos": ["https://example.com/a.mp4"]},
                      {"images": [{"url": "https://example.com/a.png"}]},
                      {"media_kwargs": {"return_tensors": "pt"}}, {"max_new_tokens": 100},
                      {"max_length": 0}, {"max_length": True},
                      {"media_kwargs": {"num_frames": 33}}, {"media_kwargs": {"fps": 121}},
                      {"media_kwargs": {"fps": 24, "num_frames": 8}},
                      {"media_kwargs": {"min_pixels": 2048, "max_pixels": 1024}}):
            with self.subTest(extra=extra), self.assertRaises(self.bridge.BadRequest):
                self.parse(body(**extra))

    def test_singleton_video_is_padded_for_the_processor_temporal_patch(self):
        request, _ = self.parse(body(videos=[{"frames": [image()]}]))
        self.assertEqual(request["videos"], [(("rgb", PNG), ("rgb", PNG))])
        with self.assertRaises(self.bridge.BadRequest):
            self.parse(body(videos=[{"frames": [image()]}],
                            media_kwargs={"do_sample_frames": True, "num_frames": 1}))

    def test_media_count_bytes_pixels_and_frame_dimensions_are_bounded(self):
        for extra in ({"images": [image()] * 5}, {"videos": [{"frames": []}]},
                      {"videos": [{"frames": [image()] * 33}]},
                      {"videos": [{"frames": [image()]}] * 5},
                      {"images": [image(b"bad png")]},
                      {"images": [image(PNG + b"x" * (4 * 1024 * 1024))]}):
            with self.subTest(keys=extra.keys()), self.assertRaises(self.bridge.BadRequest):
                self.parse(body(**extra))
        huge = bytearray(PNG)
        huge[16:24] = (5000).to_bytes(4, "big") * 2
        with self.assertRaises(self.bridge.BadRequest):
            self.parse(body(images=[image(huge)]))

    def test_model_and_primitive_validation_prevents_wrong_weights_and_empty_options(self):
        for value in (body(model="clef-flash"), body(questions={}),
                      body(questions={"x": {"type": "choice", "criteria": {"only": "a"}}}),
                      body(questions={"x": {"type": "score", "criteria": ["a"]}}),
                      body(questions={"x": {"type": "unknown"}})):
            with self.subTest(value=value), self.assertRaises(self.bridge.BadRequest):
                self.parse(value)

    def test_structured_empty_states_are_valid(self):
        for state in ({}, []):
            with self.subTest(state=state):
                request, _ = self.parse(body(state=state))
                self.assertEqual(request["state"], state)

    def test_publisher_json_scalars_and_blank_state_are_preserved(self):
        for state in (0, 1.5, True, False, None, "", " \n\t", {}, []):
            with self.subTest(state=state):
                request, _ = self.parse(body(state=state))
                self.assertIs(type(request["state"]), type(state))
                self.assertEqual(request["state"], state)

    def test_absent_state_remains_invalid_and_local_descriptions_reach_publisher(self):
        value = body()
        del value["state"]
        with self.assertRaises(self.bridge.BadRequest):
            self.parse(value)
        for instructions in (None, "", "   ", True, 42):
            with self.subTest(instructions=instructions):
                questions = {
                    "n": {"type": "noul", "instructions": instructions,
                          "criteria": {"true": None, "false": False}},
                    "c": {"type": "choice", "instructions": instructions,
                          "criteria": {"a": 0, "b": True}},
                    "s": {"type": "score", "instructions": instructions,
                          "criteria": [None, False, 1.5, ""]},
                }
                request, _ = self.parse(body(questions=questions))
                self.assertEqual(request["questions"], questions)

    def test_publisher_score_supports_large_scales_with_an_explicit_client_ceiling(self):
        for count in (26, 255):
            with self.subTest(count=count):
                levels = list(range(count))
                questions = {"scale": {"type": "score", "instructions": "Rank it.", "criteria": levels}}
                request, _ = self.parse(body(questions=questions))
                self.assertEqual(request["questions"]["scale"]["criteria"], levels)
        with self.assertRaises(self.bridge.BadRequest):
            self.parse(body(questions={"scale": {"type": "score", "criteria": list(range(256))}}))

    def test_resize_controls_cannot_amplify_many_tiny_frames_beyond_pixel_budget(self):
        # Video limits are whole-clip limits. Eight media items have an 8M
        # budget each, so a requested 16M resize cannot be admitted.
        value = body(images=[image()] * 4, videos=[{"frames": [image()] * 32}] * 4,
                     media_kwargs={"min_pixels": 16_000_000, "max_pixels": 16_000_000})
        with self.assertRaises(self.bridge.BadRequest):
            self.parse(value)
        request, _ = self.parse(body(images=[image()] * 4, videos=[{"frames": [image()] * 32}] * 4))
        self.assertEqual(request["media_kwargs"]["videos_kwargs"]["size"]["longest_edge"], 8_000_000)
        self.assertEqual(request["media_kwargs"]["images_kwargs"]["size"]["longest_edge"], 8_000_000)

    def test_default_video_budget_preserves_whole_clip_publisher_resolution(self):
        request, _ = self.parse(body(videos=[{"frames": [image()] * 32}]))
        self.assertEqual(request["media_kwargs"]["videos_kwargs"]["size"]["longest_edge"], 25_165_824)

    def test_patch_rounding_cannot_amplify_images_past_combined_budget(self):
        decoded = []
        with self.assertRaises(self.bridge.BadRequest):
            self.bridge.prepare_request(json.dumps(body(images=[image()] * 4,
                media_kwargs={"min_pixels": 16_000_000, "max_pixels": 16_000_000})).encode(),
                "clef", decode_image=lambda *args: decoded.append(args))
        self.assertEqual(decoded, [])

    def test_patch_rounding_and_odd_temporal_padding_are_checked_before_decoding(self):
        frame = bytearray(PNG)
        frame[16:24] = (64).to_bytes(4, "big") * 2
        for count, clips in ((2, 4), (3, 3)):
            decoded = []
            with self.subTest(count=count), self.assertRaises(self.bridge.BadRequest):
                self.bridge.prepare_request(json.dumps(body(videos=[{"frames": [image(frame)] * count}] * clips,
                    media_kwargs={"min_pixels": 16_000_000, "max_pixels": 16_000_000})).encode(),
                    "clef", decode_image=lambda *args: decoded.append(args))
            self.assertEqual(decoded, [])

    def test_decoder_process_control_exceptions_are_preserved(self):
        for exception in (KeyboardInterrupt, SystemExit):
            def decode(*_):
                raise exception()
            with self.subTest(exception=exception), self.assertRaises(exception):
                self.bridge.prepare_request(json.dumps(body(images=[image()])).encode(), "clef", decode_image=decode)

    def test_http_write_half_closed_client_still_receives_its_response(self):
        server = self.bridge.make_server("127.0.0.1", 0, "clef", lambda *_: {"answers": {}})
        try:
            with socket.create_connection(server.server_address, timeout=5) as client:
                raw = json.dumps(body()).encode()
                client.sendall((f"POST /v1/systemone HTTP/1.1\r\nHost: 127.0.0.1:{server.server_port}\r\n"
                    f"Content-Type: application/json\r\nContent-Length: {len(raw)}\r\n\r\n").encode() + raw)
                client.shutdown(socket.SHUT_WR)
                server.handle_request()
                response = http.client.HTTPResponse(client)
                response.begin()
                self.assertEqual(response.status, 200)
                self.assertEqual(json.loads(response.read()), {"answers": {}})
        finally:
            server.server_close()

    def test_closed_response_socket_does_not_print_a_traceback(self):
        import io
        client = None
        def infer(*_):
            linger_layout = "HH" if os.name == "nt" else "ii"
            client.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack(linger_layout, 1, 0))
            client.close()
            return {"answers": {}}
        server = self.bridge.make_server("127.0.0.1", 0, "clef", infer)
        stderr = io.StringIO()
        try:
            client = socket.create_connection(server.server_address, timeout=5)
            raw = json.dumps(body()).encode()
            client.sendall((f"POST /v1/systemone HTTP/1.1\r\nHost: 127.0.0.1:{server.server_port}\r\n"
                f"Content-Type: application/json\r\nContent-Length: {len(raw)}\r\n\r\n").encode() + raw)
            with contextlib.redirect_stderr(stderr):
                server.handle_request()
            self.assertEqual(stderr.getvalue(), "")
        finally:
            if client is not None:
                client.close()
            server.server_close()

    def test_reset_during_request_headers_does_not_print_a_traceback(self):
        import io
        calls = []
        def infer(*_):
            calls.append(True)
            return {}
        server = self.bridge.make_server("127.0.0.1", 0, "clef", infer)
        stderr = io.StringIO()
        try:
            with socket.create_connection(server.server_address, timeout=5) as client:
                client.sendall(b"POST /v1/systemone HTTP/1.1\r\nHost: private-request-canary")
                linger_layout = "HH" if os.name == "nt" else "ii"
                client.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack(linger_layout, 1, 0))
            with contextlib.redirect_stderr(stderr):
                server.handle_request()
            self.assertEqual(stderr.getvalue(), "")
            self.assertEqual(calls, [])
        finally:
            server.server_close()

    def test_http_process_control_exceptions_are_preserved(self):
        for exception in (KeyboardInterrupt, SystemExit):
            def infer(*_):
                raise exception()
            server = self.bridge.make_server("127.0.0.1", 0, "clef", infer)
            try:
                with socket.create_connection(server.server_address, timeout=5) as client:
                    raw = json.dumps(body()).encode()
                    client.sendall((f"POST /v1/systemone HTTP/1.1\r\nHost: 127.0.0.1:{server.server_port}\r\n"
                        f"Content-Type: application/json\r\nContent-Length: {len(raw)}\r\n\r\n").encode() + raw)
                    with self.subTest(exception=exception), self.assertRaises(exception):
                        server.handle_request()
            finally:
                server.server_close()

    def test_all_media_headers_are_checked_before_decoding_and_total_bytes_are_bounded(self):
        decoded = []
        def decode(data, size, kind):
            decoded.append(data)
            return data
        huge = bytearray(PNG)
        huge[16:24] = (4000).to_bytes(4, "big") * 2
        for value in (body(images=[image(PNG)], videos=[{"frames": [image(huge)] * 4}]),
                      body(images=[image(PNG + b"x" * (3 * 1024 * 1024))] * 3),
                      body(videos=[{"frames": [image(PNG), image(huge)]}])):
            with self.subTest(keys=value.keys()), self.assertRaises(self.bridge.BadRequest):
                self.bridge.prepare_request(json.dumps(value).encode(), "clef", decode_image=decode)
            self.assertEqual(decoded, [])

    def test_decoder_metadata_must_match_preflight_before_loading_pixels(self):
        loaded = []
        class Opened:
            size = (4000, 4000)
            width = height = 4000
            format = "PNG"
            n_frames = 1
            def __enter__(self):
                return self
            def __exit__(self, *_):
                pass
            def load(self):
                loaded.append(True)
                raise AssertionError("untrusted dimensions reached pixel decoding")
        fake = types.ModuleType("PIL")
        fake.Image = types.SimpleNamespace(open=lambda _: Opened(), MAX_IMAGE_PIXELS=None,
                                           DecompressionBombError=RuntimeError)
        with patch.dict(sys.modules, {"PIL": fake}):
            with self.assertRaises(self.bridge.BadRequest):
                self.bridge.prepare_request(json.dumps(body(images=[image()])).encode(), "clef")
            # Correct dimensions alone are insufficient if the claimed format differs.
            with self.assertRaises(self.bridge.BadRequest):
                self.bridge.decode_image(PNG, (4000, 4000), "image/jpeg")
        self.assertEqual(loaded, [])

    def test_pixel_conversion_does_not_retain_decompressed_container_metadata(self):
        class Converted:
            info = {"text": "container metadata does not belong in the model input"}
        class Opened:
            size = (2, 3)
            format = "PNG"
            width, height, n_frames = 2, 3, 1
            def __enter__(self):
                return self
            def __exit__(self, *_):
                pass
            def load(self):
                pass
            def convert(self, mode):
                self.mode = mode
                return Converted()
        fake = types.ModuleType("PIL")
        fake.Image = types.SimpleNamespace(open=lambda _: Opened(), MAX_IMAGE_PIXELS=None,
                                           DecompressionBombError=RuntimeError)
        with patch.dict(sys.modules, {"PIL": fake}):
            converted = self.bridge.decode_image(PNG, (2, 3), "image/png")
        self.assertEqual(converted.info, {})

    def test_decoder_bomb_exception_is_an_opaque_bad_request(self):
        class BombError(Exception):
            pass
        def opened(_):
            raise BombError("private-metadata-canary")
        fake = types.ModuleType("PIL")
        fake.Image = types.SimpleNamespace(open=opened, MAX_IMAGE_PIXELS=None,
                                           DecompressionBombError=BombError)
        with patch.dict(sys.modules, {"PIL": fake}):
            with self.assertRaises(self.bridge.BadRequest) as caught:
                self.bridge.prepare_request(json.dumps(body(images=[image()])).encode(), "clef")
        self.assertNotIn("private-metadata-canary", str(caught.exception))

    def test_independent_state_budget_composes_encoding_and_probability_answers(self):
        import contextlib
        torch = types.ModuleType("torch")
        torch.bfloat16 = object()
        torch.inference_mode = contextlib.nullcontext
        numpy = types.ModuleType("numpy")
        pil = types.ModuleType("PIL")
        pil.Image = types.ModuleType("PIL.Image")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "joint_schema_model.py").write_text('''from types import SimpleNamespace
def systemone(model, processor, request, *, max_length):
    return {"model":"clef", "answers":{"paid":{"type":"noul","noul":.875}},
            "usage":{"input_tokens":max_length,"output_tokens":0}}
class Scores:
    def float(self): return self
    def softmax(self, axis):
        assert axis == -1
        return self
    def tolist(self): return [.875, .125]
class Model:
    def parameters(self): return iter([SimpleNamespace(device="cpu")])
    def __call__(self, batch):
        assert batch == "encoded-batch"
        return [[Scores()]]
def load_release_model(path, **kwargs):
    assert kwargs["local_files_only"] is True
    return Model(), SimpleNamespace(tokenizer=SimpleNamespace(pad_token_id=7))
def encode_record(tokenizer, record, *, max_length, max_state_tokens, processor):
    assert max_length == 4096 and max_state_tokens == 128
    assert "max_state_tokens" not in record
    question = SimpleNamespace(question_id="paid", option_ids=("true","false"))
    return SimpleNamespace(input_ids=tuple(range(145)), questions=[question])
def collate_records(records, pad, device):
    assert len(records) == 1 and pad == 7 and device == "cpu"
    return "encoded-batch"
def systemone_answer(question, probabilities):
    assert question["type"] == "noul"
    assert probabilities == {"true": .875, "false": .125}
    return {"type":"noul", "noul": probabilities["true"]}
''')
            with patch.dict(sys.modules, {"torch":torch, "numpy":numpy, "PIL":pil, "PIL.Image":pil.Image}):
                infer = self.bridge.load_inference(path, "clef", "cpu", "bfloat16")
                result = infer(body(max_state_tokens=128), 4096)
            self.assertEqual(result, {"model":"clef", "answers":{"paid":{"type":"noul","noul":.875}},
                                      "usage":{"input_tokens":145,"output_tokens":0}})

    def test_explicit_local_loader_stays_offline_and_does_not_write_bytecode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "PIL").mkdir()
            (path / "PIL/__init__.py").write_text("")
            (path / "PIL/Image.py").write_text("")
            (path / "torch.py").write_text("bfloat16 = object()\n")
            (path / "numpy.py").write_text("")
            model = path / "model"
            model.mkdir()
            (model / "joint_schema_model.py").write_text('''import os, sys
def load_release_model(path, *, device, dtype, local_files_only):
    assert local_files_only is True
    assert os.environ["HF_HUB_OFFLINE"] == "1"
    assert os.environ["TRANSFORMERS_OFFLINE"] == "1"
    assert sys.dont_write_bytecode is True
    return object(), object()
def systemone(model, processor, request, *, max_length):
    return {"model": request["model"], "answers": {"paid": {"type": "noul", "noul": .875}},
            "usage": {"input_tokens": max_length, "output_tokens": 0}}
''')
            wrapper = '''import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("bridge", sys.argv[1])
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)
infer = bridge.load_inference(sys.argv[2], "clef", "cpu", "bfloat16")
server = bridge.make_server("127.0.0.1", 0, "clef", infer)
print(server.server_port, flush=True)
server.serve_forever()
'''
            env = {**os.environ, "PYTHONPATH": str(path), "HF_HUB_OFFLINE": "0", "TRANSFORMERS_OFFLINE": "0"}
            with subprocess.Popen([sys.executable, "-c", wrapper, str(SCRIPT), str(model)],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env) as child:
                try:
                    port = int(child.stdout.readline())
                    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                    connection.request("POST", "/v1/systemone", json.dumps(body(max_length=3000)),
                                       {"Content-Type": "application/json"})
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    result = json.loads(response.read())
                    self.assertEqual(result["usage"]["input_tokens"], 3000)
                    self.assertEqual(result["answers"]["paid"]["noul"], .875)
                    connection.close()
                    self.assertEqual(list(model.iterdir()), [model / "joint_schema_model.py"])
                finally:
                    child.terminate()
                    child.communicate(timeout=5)

    def test_http_routes_preserve_results_and_errors_are_opaque(self):
        def infer(request, length):
            if request["state"] == "raise":
                raise RuntimeError("private-request-canary")
            if request["state"] == "invalid":
                raise ValueError("private-request-canary: schema does not fit the token budget")
            return {"model": request["model"], "answers": {"paid": {"type": "noul", "noul": .875}},
                    "usage": {"input_tokens": length, "output_tokens": 0}}
        server = self.bridge.make_server("127.0.0.1", 0, "clef", infer)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            def call(method, path, value=None, headers=None):
                connection = http.client.HTTPConnection(*server.server_address, timeout=5)
                connection.request(method, path, json.dumps(value) if value else None,
                    headers or ({"Content-Type": "application/json"} if value else {}))
                response = connection.getresponse()
                result = response.status, dict(response.getheaders()), response.read()
                connection.close()
                return result
            status, headers, raw = call("POST", "/v1/systemone", body(max_length=2048))
            self.assertEqual(status, 200)
            self.assertEqual(json.loads(raw)["answers"]["paid"]["noul"], .875)
            self.assertEqual(json.loads(raw)["usage"]["input_tokens"], 2048)
            self.assertFalse(any(k.lower().startswith("access-control") for k in headers))
            status, _, raw = call("GET", "/v1/models")
            self.assertEqual(status, 200)
            self.assertEqual(json.loads(raw)["models"][0]["name"], "clef")
            for host in ("localhost", "localhost:" + str(server.server_port),
                         "LOCALHOST:" + str(server.server_port)):
                with self.subTest(host=host):
                    status, _, raw = call("POST", "/v1/systemone", body(),
                        {"Host": host, "Content-Type": "application/json"})
                    self.assertEqual(status, 200)
                    self.assertEqual(json.loads(raw)["answers"]["paid"]["noul"], .875)
            for method, path, value, headers, wanted in (
                ("POST", "/v1/systemone", body(state="raise"), None, 500),
                ("POST", "/v1/systemone", body(state="invalid", max_length=1), None, 400),
                ("POST", "/wrong", body(), None, 404),
                ("PUT", "/v1/systemone", body(), None, 405),
                ("POST", "/v1/systemone", body(), {"Content-Type": "text/plain"}, 415),
                ("POST", "/v1/systemone", body(), {"Origin": "https://example.com", "Content-Type": "application/json"}, 403),
                ("POST", "/v1/systemone", body(), {"Host": "attacker.example", "Content-Type": "application/json"}, 403),
                ("POST", "/v1/systemone", body(), {"Content-Type": "application/json", "Content-Length": "99999999"}, 413)):
                with self.subTest(method=method, path=path, wanted=wanted):
                    status, _, raw = call(method, path, value, headers)
                    self.assertEqual(status, wanted)
                    self.assertNotIn(b"private-request-canary", raw)
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

    def test_public_bind_and_missing_weights_fail_before_model_import(self):
        with self.assertRaises(self.bridge.BadRequest):
            self.bridge.make_server("0.0.0.0", 0, "clef", lambda *_: {})
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([sys.executable, str(SCRIPT), "--model-path", directory + "/missing"],
                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Traceback", result.stderr)
        self.assertNotIn("ModuleNotFoundError", result.stderr)


if "--real-pillow" in sys.argv:
    sys.argv.remove("--real-pillow")
    class RealPillowTests(unittest.TestCase):
        """Explicit optional dependency check; a missing Pillow installation fails."""
        def test_truncated_png_syntax_error_returns_opaque_http_400(self):
            from PIL import Image
            import io
            spec = importlib.util.spec_from_file_location("clef_server_syntax", SCRIPT)
            bridge = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(bridge)
            partial = zlib.compress(bytes(9))[:3]
            chunk = struct.pack(">I", len(partial)) + b"IDAT" + partial
            chunk += struct.pack(">I", zlib.crc32(b"IDAT" + partial))
            forged = PNG[:33] + chunk + b"\0" * 8
            with Image.open(io.BytesIO(forged)) as opened:
                self.assertEqual(opened.size, (2, 3))
                with self.assertRaises(SyntaxError):
                    opened.load()
            def infer(*_):
                raise AssertionError("malformed pixels reached inference")
            server = bridge.make_server("127.0.0.1", 0, "clef", infer)
            stderr = io.StringIO()
            try:
                with contextlib.redirect_stderr(stderr):
                    thread = threading.Thread(target=server.handle_request)
                    thread.start()
                    connection = http.client.HTTPConnection(*server.server_address, timeout=5)
                    connection.request("POST", "/v1/systemone", json.dumps(body(images=[image(forged)])),
                                       {"Content-Type": "application/json"})
                    try:
                        response = connection.getresponse()
                        self.assertEqual(response.status, 400)
                        self.assertEqual(json.loads(response.read()), {"error": "request rejected"})
                    finally:
                        connection.close()
                        thread.join()
                self.assertEqual(stderr.getvalue(), "")
            finally:
                server.server_close()
        def test_decoder_bomb_header_returns_opaque_http_400_without_inference(self):
            from PIL import Image
            import io
            spec = importlib.util.spec_from_file_location("clef_server_bomb", SCRIPT)
            bridge = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(bridge)
            def chunk(kind, data):
                return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
            header = struct.pack(">IIBBBBB", 8000, 8000, 8, 0, 0, 0, 0)
            # A header and a tiny IDAT are enough to trigger Pillow's protection;
            # this regression never constructs or decodes the advertised pixels.
            forged = (b"\x89PNG\r\n\x1a\n" + PNG[8:33] + chunk(b"IHDR", header)
                      + chunk(b"IDAT", zlib.compress(b"\0")) + chunk(b"IEND", b""))
            Image.MAX_IMAGE_PIXELS = 16_000_000
            with self.assertRaises(Image.DecompressionBombError):
                Image.open(io.BytesIO(forged))
            def infer(*_):
                raise AssertionError("invalid media reached inference")
            server = bridge.make_server("127.0.0.1", 0, "clef", infer)
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                connection = http.client.HTTPConnection(*server.server_address, timeout=5)
                connection.request("POST", "/v1/systemone", json.dumps(body(images=[image(forged)])),
                                   {"Content-Type": "application/json"})
                response = connection.getresponse()
                self.assertEqual(response.status, 400)
                self.assertEqual(json.loads(response.read()), {"error": "request rejected"})
                connection.close()
            finally:
                server.shutdown()
                thread.join()
                server.server_close()

        def test_duplicate_png_header_cannot_bypass_the_real_decoder_pixel_budget(self):
            from PIL import Image
            import io
            spec = importlib.util.spec_from_file_location("clef_server_real", SCRIPT)
            bridge = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(bridge)
            def chunk(kind, data):
                return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
            actual_header = struct.pack(">IIBBBBB", 4000, 4000, 8, 0, 0, 0, 0)
            forged = (b"\x89PNG\r\n\x1a\n" + PNG[8:33] + chunk(b"IHDR", actual_header)
                      + chunk(b"IDAT", zlib.compress(bytes(4001 * 4000))) + chunk(b"IEND", b""))
            self.assertEqual(bridge.dimensions(forged, "image/png"), (2, 3))
            with Image.open(io.BytesIO(forged)) as opened:
                self.assertEqual(opened.size, (4000, 4000))
            with self.assertRaises(bridge.BadRequest):
                bridge.prepare_request(json.dumps(body(images=[image(forged)])).encode(), "clef")


if "--real-processor" in sys.argv:
    sys.argv.remove("--real-processor")
    class RealProcessorTests(unittest.TestCase):
        """Explicit real Qwen processor check; missing model dependencies fail."""
        @classmethod
        def setUpClass(cls):
            from transformers.models.qwen3_vl.video_processing_qwen3_vl import Qwen3VLVideoProcessor
            from PIL import Image
            import io
            spec = importlib.util.spec_from_file_location("real_processor_bridge", SCRIPT)
            cls.bridge = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.bridge)
            cls.processor = Qwen3VLVideoProcessor()
            output = io.BytesIO()
            Image.new("RGB", (64, 64), (32, 64, 96)).save(output, format="PNG")
            cls.frame = image(output.getvalue())

        def process(self, value):
            prepared, _ = self.bridge.prepare_request(json.dumps(value).encode(), "clef")
            return self.processor(videos=prepared["videos"],
                **prepared["media_kwargs"]["videos_kwargs"], return_metadata=True, return_tensors="pt")

        def test_sparse_source_timestamps_and_temporal_patch_timing_are_preserved(self):
            from transformers.models.qwen3_vl.processing_qwen3_vl import Qwen3VLProcessor
            result = self.process(body(videos=[{"frames":[self.frame,self.frame],
                "metadata":{"fps":30,"total_num_frames":90,"frames_indices":[0,60],"duration":3}}],
                media_kwargs={"max_pixels":4096}))
            metadata = result["video_metadata"][0]
            self.assertEqual(metadata.timestamps, [0., 2.])
            self.assertEqual(Qwen3VLProcessor._calculate_timestamps(None, metadata.frames_indices, metadata.fps, 2), [1.])
            self.assertEqual(result["video_grid_thw"].tolist(), [[1,2,2]])

        def test_explicit_frame_count_samples_complete_array_without_default_fps_conflict(self):
            result = self.process(body(videos=[{"frames":[self.frame]*8,
                "metadata":{"fps":4,"duration":2}}],
                media_kwargs={"num_frames":4,"do_sample_frames":True,"max_pixels":4096}))
            metadata = result["video_metadata"][0]
            self.assertEqual(list(metadata.frames_indices), [0,2,5,7])
            self.assertEqual(metadata.timestamps, [0.,.5,1.25,1.75])

        def test_default_large_clip_keeps_all_frames_at_original_resolution(self):
            from PIL import Image
            import io
            output = io.BytesIO()
            Image.new("RGB", (512, 512), (32, 64, 96)).save(output, format="PNG")
            result = self.process(body(videos=[{"frames": [image(output.getvalue())] * 32}]))
            self.assertEqual(result["video_grid_thw"].tolist(), [[16, 32, 32]])

        def test_whole_clip_minimum_and_maximum_match_actual_quantized_pixels(self):
            result = self.process(body(videos=[{"frames": [self.frame] * 2}],
                media_kwargs={"min_pixels": 65536, "max_pixels": 262144}))
            self.assertEqual(result["video_grid_thw"].tolist(), [[1, 12, 12]])
            self.assertEqual(self.bridge.resized_pixels((64, 64), 2, 65536, 262144, temporal=True),
                             (36864, 73728))

        def test_maximum_video_and_frame_counts_remain_bounded_with_real_processor(self):
            result = self.process(body(videos=[{"frames": [self.frame] * 32}] * 4,
                media_kwargs={"max_pixels": 32768}))
            self.assertEqual(result["video_grid_thw"].tolist(), [[16, 2, 2]] * 4)
            # One temporal grid unit represents two 16x16 pixel patches.
            self.assertEqual(sum(t * h * w * 2 * 16 * 16
                for t, h, w in result["video_grid_thw"].tolist()), 131072)

        def test_allocation_estimate_matches_selected_real_processors(self):
            from transformers.models.qwen3_vl.video_processing_qwen3_vl import smart_resize as video_resize
            from transformers.models.qwen2_vl.image_processing_qwen2_vl import smart_resize as image_resize
            cases = ((64, 64, 4096, 4096), (512, 512, 4096, 25165824),
                     (32, 6400, 4096, 1024), (64, 96, 16000000, 16000000),
                     (65, 97, 65536, 262144))
            for width, height, minimum, maximum in cases:
                for count in (2, 3, 4, 31, 32):
                    with self.subTest(size=(width, height), count=count, minimum=minimum):
                        resized_h, resized_w = video_resize(count, height, width,
                            min_pixels=minimum, max_pixels=maximum)
                        self.assertEqual(self.bridge.resized_pixels((width, height), count,
                            minimum, maximum, temporal=True),
                            (resized_w * resized_h, (count + count % 2) * resized_w * resized_h))
                resized_h, resized_w = image_resize(height, width, factor=32,
                    min_pixels=minimum, max_pixels=maximum)
                self.assertEqual(self.bridge.resized_pixels((width, height), 1, minimum, maximum),
                                 (resized_w * resized_h, resized_w * resized_h))

        def test_downscale_rounding_boundary_never_underestimates_real_pixels(self):
            from transformers.models.qwen3_vl.video_processing_qwen3_vl import smart_resize
            self.assertEqual(smart_resize(26, 1105, 680, min_pixels=26658, max_pixels=65536), (64, 32))
            self.assertEqual(self.bridge.resized_pixels((680, 1105), 26, 26658, 65536, temporal=True),
                             (2048, 53248))


if __name__ == "__main__":
    unittest.main()
