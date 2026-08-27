#!/usr/bin/env python3
"""Tests for the pilot egress-proxy upstream-status tap (mika#1901).

WHY these exist: on 2026-08-06 a pilot session stalled 606s and was killed by
the `idle_timeout` guardrail. The proxy log said
`[anthropic-proxy] ALLOW POST /v1/messages?beta=true` — the same line it prints
for a 200 — while the call was in fact refused with HTTP 429. Establishing that
fact cost an instrumented copy of the proxy plus a host-side `curl`. These tests
fence the two properties that make the log self-sufficient:

  * the upstream status reaches the log (and a 429 gets its own greppable line);
  * the tap that reads it does not disturb the relayed byte stream.

Run standalone (`python3 scripts/test-pilot-egress-proxy-status.py`) or via
`make test`. Stdlib only — no pytest, no uv env, no network — matching the
existing script-test convention in the `test` target.
"""

from __future__ import annotations

import asyncio
import contextlib
import importlib.util
import io
import pathlib
import sys
import types
import unittest
from importlib.machinery import SourceFileLoader

# `scripts/mika-pilot-egress-proxy` has no .py extension (it is installed as a
# binary on PATH by `make install`), so it cannot be imported by name.
_PROXY_PATH = pathlib.Path(__file__).resolve().parent / "mika-pilot-egress-proxy"
_LOADER = SourceFileLoader("mika_pilot_egress_proxy", str(_PROXY_PATH))
_SPEC = importlib.util.spec_from_loader(_LOADER.name, _LOADER)
proxy = importlib.util.module_from_spec(_SPEC)
_LOADER.exec_module(proxy)


class _CapturingWriter:
    """Minimal asyncio.StreamWriter stand-in that records what it was given.

    Records each `write` separately so a test can assert on chunk boundaries,
    not just the concatenated payload.
    """

    def __init__(self) -> None:
        self.chunks: list[bytes] = []
        self.drains = 0

    def write(self, data: bytes) -> None:
        self.chunks.append(bytes(data))

    async def drain(self) -> None:
        self.drains += 1

    @property
    def payload(self) -> bytes:
        return b"".join(self.chunks)


def _reader_of(*chunks: bytes) -> asyncio.StreamReader:
    """A StreamReader pre-loaded with `chunks`, already at EOF."""
    reader = asyncio.StreamReader()
    for chunk in chunks:
        reader.feed_data(chunk)
    reader.feed_eof()
    return reader


class ParseStatusLineTests(unittest.TestCase):
    def test_complete_head_in_one_chunk(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n"
        self.assertEqual(proxy._parse_status_line(head), 200)

    def test_status_line_split_across_chunks(self) -> None:
        tap = proxy._ResponseHeadTap()
        tap.feed(b"HTTP/1.1 4")
        self.assertIsNone(proxy._parse_status_line(tap.head))
        tap.feed(b"29 Too Many Requests\r\nretry-after: 60\r\n\r\n")
        self.assertEqual(proxy._parse_status_line(tap.head), 429)

    def test_non_status_bytes_parse_to_none(self) -> None:
        self.assertIsNone(proxy._parse_status_line(b"\x00\x01garbage\r\n"))
        self.assertIsNone(proxy._parse_status_line(b""))

    def test_head_without_terminator_yields_no_status(self) -> None:
        # A single CRLF-less fragment is not yet a status line.
        self.assertIsNone(proxy._parse_status_line(b"HTTP/1.1 200 OK"))


class ResponseHeadTapTests(unittest.TestCase):
    def test_stops_accumulating_past_the_cap(self) -> None:
        tap = proxy._ResponseHeadTap(cap=64)
        tap.feed(b"HTTP/1.1 200 OK\r\n")
        tap.feed(b"x" * 512)
        self.assertTrue(tap.overflowed)
        self.assertFalse(tap.complete)
        self.assertEqual(tap.head, b"")
        self.assertIsNone(proxy._parse_status_line(tap.head))

    def test_ignores_body_bytes_after_the_head_completes(self) -> None:
        tap = proxy._ResponseHeadTap()
        tap.feed(b"HTTP/1.1 200 OK\r\nretry-after: 1\r\n\r\n")
        self.assertTrue(tap.complete)
        tap.feed(b"data: {}\r\n\r\n" * 100)
        self.assertEqual(proxy._parse_status_line(tap.head), 200)
        self.assertEqual(
            proxy._select_quota_headers(tap.head), [("retry-after", "1")]
        )


class QuotaHeaderSelectionTests(unittest.TestCase):
    def test_selects_allowlisted_headers_preserving_upstream_casing(self) -> None:
        head = (
            b"HTTP/1.1 429 Too Many Requests\r\n"
            b"Content-Type: application/json\r\n"
            b"Retry-After: 42\r\n"
            b"anthropic-ratelimit-requests-remaining: 0\r\n"
            b"request-id: req_011CdmbTL5FH62zfwP7ieMhu\r\n"
            b"\r\n"
        )
        self.assertEqual(
            proxy._select_quota_headers(head),
            [
                ("Retry-After", "42"),
                ("anthropic-ratelimit-requests-remaining", "0"),
                ("request-id", "req_011CdmbTL5FH62zfwP7ieMhu"),
            ],
        )

    def test_rejects_everything_outside_the_allowlist(self) -> None:
        # R10 by construction: an Authorization header must not survive
        # selection, whatever the upstream sends back.
        head = (
            b"HTTP/1.1 200 OK\r\n"
            b"content-type: application/json\r\n"
            b"authorization: Bearer sk-ant-oat01-SECRET\r\n"
            b"set-cookie: session=abc\r\n"
            b"\r\n"
        )
        self.assertEqual(proxy._select_quota_headers(head), [])

    def test_status_line_is_never_mistaken_for_a_header(self) -> None:
        head = b"HTTP/1.1 429 Too Many Requests\r\n\r\n"
        self.assertEqual(proxy._select_quota_headers(head), [])

    def test_formats_pairs_without_leaking_separators(self) -> None:
        rendered = proxy._format_quota_headers(
            [("Retry-After", "42"), ("request-id", "req_abc")]
        )
        self.assertEqual(rendered, "Retry-After=42 request-id=req_abc")
        self.assertEqual(proxy._format_quota_headers([]), "")


class RelayTapTests(unittest.IsolatedAsyncioTestCase):
    async def test_relays_multi_chunk_sse_byte_identically(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
            b"event: message_start\r\ndata: {}\r\n\r\n",
            b"event: content_block_delta\r\ndata: {}\r\n\r\n",
            b"event: message_stop\r\ndata: {}\r\n\r\n",
        ]
        writer = _CapturingWriter()
        status, headers = await proxy._relay_response_with_status_tap(
            _reader_of(*chunks), writer
        )
        self.assertEqual(status, 200)
        self.assertEqual(headers, [])
        self.assertEqual(writer.payload, b"".join(chunks))

    async def test_head_split_mid_status_line_still_relays_intact(self) -> None:
        chunks = [b"HTTP/1.", b"1 429 Too Many\r\nRetry-After: 7\r\n\r\n", b"{}"]
        writer = _CapturingWriter()
        status, headers = await proxy._relay_response_with_status_tap(
            _reader_of(*chunks), writer
        )
        self.assertEqual(status, 429)
        self.assertEqual(headers, [("Retry-After", "7")])
        self.assertEqual(writer.payload, b"".join(chunks))

    async def test_upstream_closing_with_no_bytes_reports_no_status(self) -> None:
        writer = _CapturingWriter()
        status, headers = await proxy._relay_response_with_status_tap(
            _reader_of(), writer
        )
        self.assertIsNone(status)
        self.assertEqual(headers, [])
        self.assertEqual(writer.payload, b"")

    async def test_client_receives_each_chunk_before_the_next_is_read(self) -> None:
        # The tap must not batch: a streamed response has to reach the client
        # as it arrives, or the pilot's SSE parser stalls waiting for bytes the
        # proxy is holding. Each upstream read is followed by a drain.
        chunks = [
            b"HTTP/1.1 200 OK\r\n\r\n",
            b"data: one\r\n\r\n",
            b"data: two\r\n\r\n",
        ]
        writer = _CapturingWriter()
        await proxy._relay_response_with_status_tap(_reader_of(*chunks), writer)
        self.assertEqual(writer.drains, len(writer.chunks))
        self.assertGreaterEqual(writer.drains, 1)


class UpstreamOutcomeLoggingTests(unittest.TestCase):
    """The lines an operator greps. These are the product of mika#1901."""

    def _emit(self, status, quota=()) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            proxy._log_upstream_outcome("POST", "/v1/messages?beta=true", status, list(quota))
        return buffer.getvalue().splitlines()

    def test_success_logs_one_allow_line_carrying_the_status(self) -> None:
        lines = self._emit(200)
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    def test_throttle_logs_allow_plus_a_distinct_rate_limited_line(self) -> None:
        lines = self._emit(
            429,
            [("Retry-After", "42"), ("request-id", "req_011CdmbTL5FH62zfwP7ieMhu")],
        )
        self.assertEqual(
            lines,
            [
                "[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 429",
                "[anthropic-proxy] RATE_LIMITED POST /v1/messages?beta=true "
                "Retry-After=42 request-id=req_011CdmbTL5FH62zfwP7ieMhu",
            ],
        )

    def test_throttle_without_quota_headers_still_names_the_class(self) -> None:
        lines = self._emit(429)
        self.assertEqual(
            lines[1], "[anthropic-proxy] RATE_LIMITED POST /v1/messages?beta=true"
        )
        self.assertFalse(lines[1].endswith(" "))

    def test_other_error_statuses_surface_without_a_rate_limited_line(self) -> None:
        for status in (401, 500, 529):
            with self.subTest(status=status):
                lines = self._emit(status)
                self.assertEqual(len(lines), 1)
                self.assertTrue(lines[0].endswith(f"-> {status}"))

    def test_upstream_that_never_answered_is_not_reported_as_allowed(self) -> None:
        # The 2026-08-06 shape: STREAM_START with no STREAM_END. Reporting this
        # as ALLOW is what sent the investigation toward the streaming layer.
        lines = self._emit(None)
        self.assertEqual(
            lines,
            ["[anthropic-proxy] UPSTREAM_NO_RESPONSE POST /v1/messages?beta=true"],
        )
        self.assertNotIn("ALLOW", lines[0])


class LogSecrecyTests(unittest.TestCase):
    """R10: no log line may carry the token, an auth header, or a body byte."""

    SECRET = "sk-ant-oat01-DO-NOT-LOG-ME"

    def test_no_emitted_line_can_carry_upstream_secrets_or_body(self) -> None:
        head = (
            b"HTTP/1.1 429 Too Many Requests\r\n"
            b"authorization: Bearer " + self.SECRET.encode() + b"\r\n"
            b"set-cookie: session=abc\r\n"
            b"Retry-After: 9\r\n"
            b"\r\n"
        )
        body = b'{"error":{"message":"' + self.SECRET.encode() + b'"}}'
        status = proxy._parse_status_line(head)
        quota = proxy._select_quota_headers(head)

        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            proxy._log_upstream_outcome("POST", "/v1/messages", status, quota)
        emitted = buffer.getvalue()

        self.assertIn("RATE_LIMITED", emitted)
        self.assertIn("Retry-After=9", emitted)
        self.assertNotIn(self.SECRET, emitted)
        self.assertNotIn("Bearer", emitted)
        self.assertNotIn("set-cookie", emitted)
        self.assertNotIn(body.decode(), emitted)


# ---------------------------------------------------------------------------
# mitmproxy addon (the CONNECT path)
#
# The addon imports `mitmproxy`, which is a pilot-host dependency and is not
# installed where the test suite runs. Stub the two names it touches so the
# module is importable here; the stub is never used by the addon's logic, only
# by its import line.
# ---------------------------------------------------------------------------

_MITM_STUB = types.ModuleType("mitmproxy")
_MITM_HTTP_STUB = types.ModuleType("mitmproxy.http")
_MITM_HTTP_STUB.HTTPFlow = type("HTTPFlow", (), {})
_MITM_HTTP_STUB.Response = type("Response", (), {"make": staticmethod(lambda *a, **k: None)})
_MITM_STUB.http = _MITM_HTTP_STUB
sys.modules.setdefault("mitmproxy", _MITM_STUB)
sys.modules.setdefault("mitmproxy.http", _MITM_HTTP_STUB)

_ADDON_PATH = pathlib.Path(__file__).resolve().parent / "mika-pilot-anthropic-auth-addon.py"
_ADDON_LOADER = SourceFileLoader("mika_pilot_anthropic_auth_addon", str(_ADDON_PATH))
_ADDON_SPEC = importlib.util.spec_from_loader(_ADDON_LOADER.name, _ADDON_LOADER)
addon = importlib.util.module_from_spec(_ADDON_SPEC)
_ADDON_LOADER.exec_module(addon)


class _FakeHeaders:
    """Case-insensitive header view with the two methods the addon may use."""

    def __init__(self, pairs: list[tuple[str, str]]) -> None:
        self._pairs = pairs

    def items(self):
        return list(self._pairs)

    def get(self, name, default=None):
        for key, value in self._pairs:
            if key.lower() == name.lower():
                return value
        return default


class _TripwireResponse:
    """A response whose body access is a test failure.

    R9 says the addon must not read, buffer, or alter the body. Making
    `.content` raise turns that from a review promise into a test.
    """

    def __init__(self, status_code: int, headers: list[tuple[str, str]]) -> None:
        self.status_code = status_code
        self.headers = _FakeHeaders(headers)
        self.stream = False

    @property
    def content(self):  # pragma: no cover - the raise IS the assertion
        raise AssertionError("addon touched flow.response.content (violates R9)")

    @property
    def text(self):  # pragma: no cover - same
        raise AssertionError("addon touched flow.response.text (violates R9)")


class _FakeFlow:
    def __init__(self, host: str, response: _TripwireResponse | None) -> None:
        self.request = types.SimpleNamespace(
            host=host, method="POST", path="/v1/messages?beta=true"
        )
        self.response = response


class AddonResponseLoggingTests(unittest.TestCase):
    def _emit(self, host: str, response: _TripwireResponse | None) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            addon.responseheaders(_FakeFlow(host, response))
        return buffer.getvalue().splitlines()

    def test_success_logs_the_status(self) -> None:
        lines = self._emit(
            "api.anthropic.com", _TripwireResponse(200, [("content-type", "text/json")])
        )
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    def test_throttle_logs_the_same_class_line_as_the_reverse_proxy(self) -> None:
        lines = self._emit(
            "api.anthropic.com",
            _TripwireResponse(
                429,
                [
                    ("Retry-After", "30"),
                    ("anthropic-ratelimit-requests-remaining", "0"),
                    ("authorization", "Bearer sk-ant-oat01-SECRET"),
                ],
            ),
        )
        self.assertEqual(lines[0].split(" -> ")[-1], "429")
        self.assertTrue(lines[1].startswith("[anthropic-proxy] RATE_LIMITED POST "))
        self.assertIn("Retry-After=30", lines[1])
        self.assertIn("anthropic-ratelimit-requests-remaining=0", lines[1])
        self.assertNotIn("SECRET", lines[1])
        self.assertNotIn("Bearer", lines[1])

    def test_non_anthropic_flow_logs_nothing(self) -> None:
        self.assertEqual(
            self._emit("github.com", _TripwireResponse(429, [("Retry-After", "1")])), []
        )

    def test_missing_response_is_survivable(self) -> None:
        self.assertEqual(self._emit("api.anthropic.com", None), [])

    def test_addon_does_not_enable_response_streaming(self) -> None:
        # Setting flow.response.stream here would change mitmproxy's body
        # handling — the thing R9 exists to prevent.
        response = _TripwireResponse(200, [])
        with contextlib.redirect_stderr(io.StringIO()):
            addon.responseheaders(_FakeFlow("api.anthropic.com", response))
        self.assertFalse(response.stream)


if __name__ == "__main__":
    unittest.main(verbosity=2, argv=[sys.argv[0]])
