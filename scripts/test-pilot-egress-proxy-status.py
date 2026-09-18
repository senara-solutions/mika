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
import datetime
import importlib.util
import io
import os
import pathlib
import re
import signal
import socket
import subprocess
import tempfile
import time
import sys
import types
import unittest
from importlib.machinery import SourceFileLoader

# mika#2030: every line the proxy writes to `pilot-egress-proxy.log` now leads
# with an ISO-8601 UTC millisecond stamp + a single space. This is the shape
# `server.log`'s JSON `timestamp` uses (RFC3339 UTC), truncated to ms, so a
# `grep 2026-08-28` composes across both files. The tests below assert the
# stamp is present and parseable, then strip it so the message-shape assertions
# established by mika#1901 keep reading against the un-prefixed text.
_TS_PREFIX_RE = re.compile(
    r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z) (?P<rest>.*)$"
)


def _strip_ts(test: unittest.TestCase, lines: list[str]) -> list[str]:
    """Assert every non-empty line is timestamped + parseable, return the
    message bodies with the stamp removed. Enforcing this on the shared test
    helpers makes AC1 (all lines stamped) hold across every logging path these
    tests already exercise, not just the ones that name the timestamp."""
    stripped: list[str] = []
    for line in lines:
        if not line:
            continue
        match = _TS_PREFIX_RE.match(line)
        test.assertIsNotNone(match, f"log line is not timestamped: {line!r}")
        assert match is not None  # for type-checkers; assertIsNotNone already failed
        # A stamp that is merely shaped right is not enough — it must parse.
        datetime.datetime.strptime(match.group("ts"), "%Y-%m-%dT%H:%M:%S.%fZ")
        stripped.append(match.group("rest"))
    return stripped

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


def _held_open_reader_of(*chunks: bytes) -> asyncio.StreamReader:
    """A StreamReader pre-loaded with `chunks` that NEVER reaches EOF.

    This is the whole condition of mika#2317: an upstream that answered in full
    and then kept the connection open. Before the fix the relay sat on
    `up_reader.read()` here until the upstream's idle-timeout (~360 s measured
    on 2026-08-2x), so any test using this reader hangs on the old code and is
    bounded by `asyncio.wait_for` — the expiry IS the failure.
    """
    reader = asyncio.StreamReader()
    for chunk in chunks:
        reader.feed_data(chunk)
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

    def test_unterminated_first_line_still_yields_its_code(self) -> None:
        # The code is readable before the CRLF arrives; requiring the CRLF is
        # what made a headerless response report as UPSTREAM_NO_RESPONSE.
        self.assertEqual(proxy._parse_status_line(b"HTTP/1.1 200 OK"), 200)

    def test_status_line_with_no_headers_at_all_is_read(self) -> None:
        tap = proxy._ResponseHeadTap()
        tap.feed(b"HTTP/1.1 429 Too Many Requests\r\n\r\n")
        self.assertEqual(proxy._parse_status_line(tap.head), 429)

    def test_partially_arrived_code_is_not_guessed(self) -> None:
        self.assertIsNone(proxy._parse_status_line(b"HTTP/1.1 4"))
        self.assertIsNone(proxy._parse_status_line(b"HTTP/1.1 42"))
        # A prefix of a longer number must not be read as a 3-digit code.
        self.assertIsNone(proxy._parse_status_line(b"HTTP/1.1 4299 Weird"))


class ResponseHeadTapTests(unittest.TestCase):
    def test_stops_accumulating_past_the_cap(self) -> None:
        tap = proxy._ResponseHeadTap(cap=64)
        tap.feed(b"HTTP/1.1 200 OK\r\n")
        tap.feed(b"x" * 512)
        self.assertTrue(tap.overflowed)
        self.assertFalse(tap.complete)
        self.assertEqual(tap.head, b"")
        self.assertIsNone(proxy._parse_status_line(tap.head))

    def test_interim_1xx_head_does_not_become_the_verdict(self) -> None:
        # 100 Continue / 103 Early Hints are preambles. Latching on one would
        # log `-> 100` and hide the real answer behind it.
        tap = proxy._ResponseHeadTap()
        tap.feed(
            b"HTTP/1.1 100 Continue\r\n\r\n"
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 5\r\n\r\n"
        )
        self.assertTrue(tap.complete)
        self.assertEqual(proxy._parse_status_line(tap.head), 429)
        self.assertEqual(proxy._select_quota_headers(tap.head), [("Retry-After", "5")])

    def test_interim_head_split_across_chunks_is_skipped(self) -> None:
        tap = proxy._ResponseHeadTap()
        tap.feed(b"HTTP/1.1 103 Early Hints\r\nlink: </x>\r\n\r")
        self.assertFalse(tap.complete)
        tap.feed(b"\nHTTP/1.1 200 OK\r\n\r\n")
        self.assertTrue(tap.complete)
        self.assertEqual(proxy._parse_status_line(tap.head), 200)

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
    """The relay owns byte fidelity AND the one-line-per-request guarantee."""

    async def _relay(self, reader, writer):
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            await proxy._relay_response_with_status_tap(
                reader, writer, "POST", "/v1/messages?beta=true"
            )
        return _strip_ts(self, buffer.getvalue().splitlines())

    async def test_relays_multi_chunk_sse_byte_identically(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
            b"event: message_start\r\ndata: {}\r\n\r\n",
            b"event: content_block_delta\r\ndata: {}\r\n\r\n",
            b"event: message_stop\r\ndata: {}\r\n\r\n",
        ]
        writer = _CapturingWriter()
        lines = await self._relay(_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    async def test_head_split_mid_status_line_still_relays_intact(self) -> None:
        chunks = [b"HTTP/1.", b"1 429 Too Many\r\nRetry-After: 7\r\n\r\n", b"{}"]
        writer = _CapturingWriter()
        lines = await self._relay(_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertIn("Retry-After=7", lines[1])

    async def test_upstream_closing_with_no_bytes_reports_no_status(self) -> None:
        writer = _CapturingWriter()
        lines = await self._relay(_reader_of(), writer)
        self.assertEqual(writer.payload, b"")
        self.assertEqual(
            lines,
            ["[anthropic-proxy] UPSTREAM_NO_RESPONSE POST /v1/messages?beta=true"],
        )

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
        await self._relay(_reader_of(*chunks), writer)
        self.assertEqual(writer.drains, len(writer.chunks))
        self.assertGreaterEqual(writer.drains, 1)

    async def test_verdict_survives_a_client_that_dies_mid_body(self) -> None:
        # The founding incident's real shape: upstream answers 429, the pilot
        # stalls, the guardrail kills it, the client socket closes. Logging
        # only after a clean EOF loses the verdict exactly here.
        class DyingWriter(_CapturingWriter):
            def __init__(self, die_after: int) -> None:
                super().__init__()
                self._die_after = die_after

            async def drain(self) -> None:
                await super().drain()
                if self.drains > self._die_after:
                    raise ConnectionResetError("client gone")

        writer = DyingWriter(die_after=1)
        reader = _reader_of(
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\n\r\n",
            b'{"error":"rate_limit"}',
        )
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            with contextlib.suppress(ConnectionResetError):
                await proxy._relay_response_with_status_tap(
                    reader, writer, "POST", "/v1/messages"
                )
        lines = buffer.getvalue().splitlines()
        self.assertTrue(any("RATE_LIMITED" in line for line in lines), lines)

    async def test_stalled_stream_reports_before_the_body_ends(self) -> None:
        # The verdict is emitted at the head, so a response that never
        # completes is still diagnosed.
        reader = asyncio.StreamReader()
        reader.feed_data(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\n\r\n")
        writer = _CapturingWriter()
        buffer = io.StringIO()

        async def relay():
            with contextlib.redirect_stderr(buffer):
                await proxy._relay_response_with_status_tap(
                    reader, writer, "POST", "/v1/messages"
                )

        task = asyncio.create_task(relay())
        await asyncio.sleep(0.05)  # upstream is still "thinking"; no EOF yet
        self.assertIn("RATE_LIMITED", buffer.getvalue())
        reader.feed_eof()
        await task
        # And exactly one verdict, not one per phase.
        self.assertEqual(buffer.getvalue().count("RATE_LIMITED"), 1)
        self.assertEqual(buffer.getvalue().count("ALLOW"), 1)

    async def test_interim_response_does_not_produce_an_early_verdict(self) -> None:
        chunks = [
            b"HTTP/1.1 100 Continue\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
            b"data: one\r\n\r\n",
        ]
        writer = _CapturingWriter()
        lines = await self._relay(_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )


# ---------------------------------------------------------------------------
# Closing on the end of the BODY, not on upstream EOF (mika#2317)
#
# mika#2313 fixed the ~358 s-per-turn hang by no longer forwarding the client's
# `Connection: keep-alive` upstream. That works because the upstream HONOURS our
# `Connection: close` — a policy, not a property of the relay. These tests fence
# the defence in depth: the relay establishes the end of the body itself and
# returns there, so an upstream that one day ignored `close` could not bring the
# hang back.
#
# The reader used below never reaches EOF. On the pre-fix relay every one of
# these tests hangs; `asyncio.wait_for` is what converts that hang into a
# failure instead of a stuck suite.
# ---------------------------------------------------------------------------

_BODY_END_TIMEOUT = 2.0


class BodyEndClosureTests(unittest.IsolatedAsyncioTestCase):
    async def _relay_until_body_end(
        self, reader, writer, method: str = "POST"
    ) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            await asyncio.wait_for(
                proxy._relay_response_with_status_tap(
                    reader, writer, method, "/v1/messages?beta=true"
                ),
                timeout=_BODY_END_TIMEOUT,
            )
        return _strip_ts(self, buffer.getvalue().splitlines())

    async def _assert_waits_for_eof(self, reader, writer, method: str = "POST") -> None:
        """The relay must NOT return: no end of body was proven."""
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(asyncio.TimeoutError):
                await asyncio.wait_for(
                    proxy._relay_response_with_status_tap(
                        reader, writer, method, "/v1/messages"
                    ),
                    timeout=0.25,
                )

    # --- U2: Content-Length ------------------------------------------------

    async def test_content_length_body_closes_without_an_upstream_eof(self) -> None:
        # The ticket's literal criterion: a complete response on a connection
        # the upstream keeps open must still close the client immediately after
        # the last byte of the body.
        chunks = [
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n"
            b"content-length: 21\r\n\r\n",
            b'{"content":"hello!!"}',
        ]
        writer = _CapturingWriter()
        lines = await self._relay_until_body_end(
            _held_open_reader_of(*chunks), writer
        )
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    async def test_content_length_body_split_across_reads_still_closes(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 12\r\n\r\n"
        chunks = [head, b"hello", b" ", b"world!"]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))

    async def test_body_that_starts_in_the_head_chunk_is_counted(self) -> None:
        # The head terminator and the first body bytes arrive in ONE read. If
        # the relay did not subtract the head, it would wait for 5 more bytes
        # that never come — a silent false negative, invisible until an upstream
        # stops closing.
        one = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello"
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(one), writer)
        self.assertEqual(writer.payload, one)

    # --- U3 / U4: chunked --------------------------------------------------

    async def test_chunked_body_closes_at_the_decoded_terminator(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n",
            b"5\r\nhello\r\n",
            b"6\r\n world\r\n",
            b"0\r\n\r\n",
        ]
        writer = _CapturingWriter()
        lines = await self._relay_until_body_end(
            _held_open_reader_of(*chunks), writer
        )
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    async def test_chunked_terminator_inside_chunk_data_is_not_an_end(self) -> None:
        # THE test of this ticket (plan R1). `0\r\n\r\n` is not a substring to
        # search for: it occurs inside chunk data, which here is arbitrary SSE
        # JSON. An `in chunk` predicate — the one the request path still uses at
        # the other end of this file — would cut a live LLM stream right here.
        poison = b'data: {"text":"0\r\n\r\n"}'
        chunks = [
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n",
            f"{len(poison):x}\r\n".encode("ascii") + poison + b"\r\n",
            b"9\r\nafterward\r\n",
            b"0\r\n\r\n",
        ]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(
            writer.payload,
            b"".join(chunks),
            "the relay cut the body at a substring inside chunk data",
        )

    async def test_chunked_extensions_and_trailers_are_decoded(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"5;name=value\r\nhello\r\n",
            b"0\r\n",
            b"x-checksum: abc\r\n",
            b"\r\n",
        ]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))

    async def test_gzip_then_chunked_is_still_chunked_framed(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: gzip, chunked\r\n\r\n",
            b"3\r\nabc\r\n0\r\n\r\n",
        ]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))

    # --- U5: malformed chunked disarms, it does not guess ------------------

    async def test_non_hex_chunk_size_falls_back_to_the_eof_loop(self) -> None:
        held = _held_open_reader_of(
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n",
            b"zz\r\ngarbage\r\n0\r\n\r\n",
        )
        await self._assert_waits_for_eof(held, _CapturingWriter())

    async def test_missing_crlf_after_chunk_data_falls_back_to_the_eof_loop(
        self,
    ) -> None:
        held = _held_open_reader_of(
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n",
            b"5\r\nhelloXX0\r\n\r\n",  # data not followed by CRLF
        )
        await self._assert_waits_for_eof(held, _CapturingWriter())

    async def test_malformed_chunked_still_relays_every_byte_to_the_client(
        self,
    ) -> None:
        # Disarming must cost fidelity nothing: with the upstream closing, the
        # client receives the whole stream exactly as before this work.
        chunks = [
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n",
            b"zz\r\ngarbage bytes\r\n",
        ]
        writer = _CapturingWriter()
        lines = await self._relay_until_body_end(_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    # --- U7: no body at all ------------------------------------------------

    async def test_content_length_zero_ends_at_the_head(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n"
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(head), writer)
        self.assertEqual(writer.payload, head)

    async def test_204_ends_at_the_head(self) -> None:
        head = b"HTTP/1.1 204 No Content\r\n\r\n"
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(head), writer)
        self.assertEqual(writer.payload, head)

    async def test_head_request_response_ends_at_the_head(self) -> None:
        # A HEAD response carries the Content-Length of the body it does not
        # send. Waiting for those bytes would wait forever.
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 4096\r\n\r\n"
        writer = _CapturingWriter()
        await self._relay_until_body_end(
            _held_open_reader_of(head), writer, method="HEAD"
        )
        self.assertEqual(writer.payload, head)

    # --- U9: still exactly one verdict -------------------------------------

    async def test_early_close_still_logs_exactly_one_verdict(self) -> None:
        chunks = [
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\n"
            b"content-length: 22\r\n\r\n",
            b'{"error":"rate_limit"}',
        ]
        writer = _CapturingWriter()
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            await asyncio.wait_for(
                proxy._relay_response_with_status_tap(
                    _held_open_reader_of(*chunks), writer, "POST", "/v1/messages"
                ),
                timeout=_BODY_END_TIMEOUT,
            )
        emitted = buffer.getvalue()
        self.assertEqual(emitted.count("ALLOW"), 1, emitted)
        self.assertEqual(emitted.count("RATE_LIMITED"), 1, emitted)
        self.assertNotIn("UPSTREAM_NO_RESPONSE", emitted)

    # --- U10: framing is insensitive to packet boundaries ------------------

    async def test_head_and_chunk_size_split_across_reads(self) -> None:
        chunks = [
            b"HTTP/1.1 200 OK\r\ntransfer-enc",          # mid header name
            b"oding: chunked\r\n\r",                     # mid head terminator
            b"\n1",                                      # mid chunk-size line
            b"2\r\nhello big world!!!\r\n",               # 0x12 == 18 bytes
            b"0\r",
            b"\n\r\n",
        ]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))

    async def test_content_length_head_split_mid_header_value(self) -> None:
        chunks = [b"HTTP/1.1 200 OK\r\ncontent-len", b"gth: 4\r\n\r\nabcd"]
        writer = _CapturingWriter()
        await self._relay_until_body_end(_held_open_reader_of(*chunks), writer)
        self.assertEqual(writer.payload, b"".join(chunks))

    # --- U8: the 1xx preamble must be counted in the offset ----------------

    async def test_interim_preamble_does_not_shift_the_body_count(self) -> None:
        # Without D4's offset counting the preamble, the body would be declared
        # over exactly `len(preamble)` bytes early: a silent false positive that
        # truncates the client's stream. Here the whole response arrives in one
        # read, so a mis-count would break the payload assertion below.
        one = (
            b"HTTP/1.1 100 Continue\r\n\r\n"
            b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\n"
            b"streaming"
        )
        writer = _CapturingWriter()
        lines = await self._relay_until_body_end(_held_open_reader_of(one), writer)
        self.assertEqual(writer.payload, one)
        self.assertEqual(
            lines, ["[anthropic-proxy] ALLOW POST /v1/messages?beta=true -> 200"]
        )

    # --- AC4: nothing provable, nothing closed -----------------------------

    async def test_response_without_framing_headers_waits_for_eof(self) -> None:
        held = _held_open_reader_of(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
            b"data: one\r\n\r\n",
        )
        await self._assert_waits_for_eof(held, _CapturingWriter())

    async def test_overflowing_head_waits_for_eof(self) -> None:
        # The padding carries NO head terminator, so the tap overflows before
        # ever finding one: where the body begins is unknowable, and a
        # `Content-Length` read out of a head we never finished reading would
        # be a guess. The relay must stay on the EOF loop.
        held = _held_open_reader_of(
            b"HTTP/1.1 200 OK\r\ncontent-length: 1\r\n",
            # Past the cap by a whole read: the tap only gives up once its
            # buffer exceeds the cap, and one read is capped at BUFFER_SIZE.
            b"x-pad: " + b"p" * (proxy.RESPONSE_HEAD_CAP * 2),
            b"\r\n\r\ny",
        )
        await self._assert_waits_for_eof(held, _CapturingWriter())

    async def test_content_length_and_transfer_encoding_together_wait_for_eof(
        self,
    ) -> None:
        held = _held_open_reader_of(
            b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n"
            b"transfer-encoding: chunked\r\n\r\n",
            b"hello",
        )
        await self._assert_waits_for_eof(held, _CapturingWriter())


class ResponseFramingClassificationTests(unittest.TestCase):
    """U6 — one case per row of the plan's arming table (D2)."""

    def _classify(self, head: bytes, method: str = "POST"):
        return proxy._classify_response_framing(head, method)

    def test_bodiless_statuses_and_head_requests(self) -> None:
        for head, method in (
            (b"HTTP/1.1 204 No Content\r\n\r\n", "POST"),
            (b"HTTP/1.1 304 Not Modified\r\netag: x\r\n\r\n", "GET"),
            (b"HTTP/1.1 200 OK\r\ncontent-length: 99\r\n\r\n", "HEAD"),
            (b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n", "head"),
        ):
            with self.subTest(head=head, method=method):
                self.assertEqual(
                    self._classify(head, method), (proxy._FRAMING_NO_BODY, 0)
                )

    def test_content_length_and_transfer_encoding_together_are_undetermined(
        self,
    ) -> None:
        head = (
            b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n"
            b"transfer-encoding: chunked\r\n\r\n"
        )
        self.assertEqual(self._classify(head), (proxy._FRAMING_UNDETERMINED, 0))

    def test_transfer_encoding_ending_in_chunked_is_chunked(self) -> None:
        for value in (b"chunked", b"gzip, chunked", b"Chunked", b"gzip,chunked "):
            with self.subTest(value=value):
                head = b"HTTP/1.1 200 OK\r\ntransfer-encoding: " + value + b"\r\n\r\n"
                self.assertEqual(self._classify(head), (proxy._FRAMING_CHUNKED, 0))

    def test_transfer_encoding_not_ending_in_chunked_is_undetermined(self) -> None:
        for value in (b"gzip", b"chunked, gzip", b"", b"identity"):
            with self.subTest(value=value):
                head = b"HTTP/1.1 200 OK\r\ntransfer-encoding: " + value + b"\r\n\r\n"
                self.assertEqual(
                    self._classify(head), (proxy._FRAMING_UNDETERMINED, 0)
                )

    def test_single_content_length_is_a_length(self) -> None:
        head = b"HTTP/1.1 200 OK\r\nContent-Length: 1234\r\n\r\n"
        self.assertEqual(self._classify(head), (proxy._FRAMING_LENGTH, 1234))

    def test_zero_content_length_is_a_length_of_zero(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n"
        self.assertEqual(self._classify(head), (proxy._FRAMING_LENGTH, 0))

    def test_repeated_identical_content_length_is_accepted(self) -> None:
        head = (
            b"HTTP/1.1 200 OK\r\ncontent-length: 7\r\ncontent-length: 7\r\n\r\n"
        )
        self.assertEqual(self._classify(head), (proxy._FRAMING_LENGTH, 7))

    def test_divergent_content_lengths_are_undetermined(self) -> None:
        for head in (
            b"HTTP/1.1 200 OK\r\ncontent-length: 7\r\ncontent-length: 9\r\n\r\n",
            b"HTTP/1.1 200 OK\r\ncontent-length: 7, 9\r\n\r\n",
        ):
            with self.subTest(head=head):
                self.assertEqual(
                    self._classify(head), (proxy._FRAMING_UNDETERMINED, 0)
                )

    def test_unparseable_content_length_is_undetermined(self) -> None:
        for value in (b"abc", b"", b"-1", b"12.5", b"0x10"):
            with self.subTest(value=value):
                head = b"HTTP/1.1 200 OK\r\ncontent-length: " + value + b"\r\n\r\n"
                self.assertEqual(
                    self._classify(head), (proxy._FRAMING_UNDETERMINED, 0)
                )

    def test_neither_header_is_undetermined(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n"
        self.assertEqual(self._classify(head), (proxy._FRAMING_UNDETERMINED, 0))

    def test_an_unreadable_head_never_claims_no_body(self) -> None:
        # The overflow path constructs this tap explicitly; asserting it here
        # keeps the `undetermined()` constructor from quietly becoming
        # `_BodyFramingTap(b"", method)`, which for HEAD would say NO_BODY on a
        # head whose end was never found.
        tap = proxy._BodyFramingTap.undetermined()
        self.assertEqual(tap.mode, proxy._FRAMING_UNDETERMINED)
        self.assertFalse(tap.done)
        self.assertFalse(tap.determinate)
        tap.feed(b"anything at all")
        self.assertFalse(tap.done)


class ResponseHeadEndOffsetTests(unittest.TestCase):
    """U8 — D4's counting, isolated from the relay."""

    def test_offset_is_none_until_the_head_completes(self) -> None:
        tap = proxy._ResponseHeadTap()
        tap.feed(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n")
        self.assertIsNone(tap.head_end_offset)

    def test_offset_covers_the_head_terminator(self) -> None:
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n"
        tap = proxy._ResponseHeadTap()
        tap.feed(head + b"hi")
        self.assertEqual(tap.head_end_offset, len(head))

    def test_offset_counts_discarded_1xx_preambles(self) -> None:
        preamble = b"HTTP/1.1 100 Continue\r\n\r\n"
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n"
        tap = proxy._ResponseHeadTap()
        tap.feed(preamble + head + b"hi")
        self.assertEqual(tap.head_end_offset, len(preamble) + len(head))

    def test_offset_survives_a_head_split_across_feeds(self) -> None:
        preamble = b"HTTP/1.1 103 Early Hints\r\nlink: </x>\r\n\r\n"
        head = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n"
        whole = preamble + head
        tap = proxy._ResponseHeadTap()
        for index in range(0, len(whole), 7):
            tap.feed(whole[index:index + 7])
        self.assertEqual(tap.head_end_offset, len(whole))

    def test_overflow_leaves_the_offset_unknown(self) -> None:
        tap = proxy._ResponseHeadTap(cap=64)
        tap.feed(b"HTTP/1.1 200 OK\r\n" + b"x" * 512)
        self.assertTrue(tap.overflowed)
        self.assertIsNone(tap.head_end_offset)


class RelayTerminationCountersTests(unittest.IsolatedAsyncioTestCase):
    """AC7 — the classes are counted, and by default nothing is printed."""

    def setUp(self) -> None:
        self._saved = dict(proxy._relay_termination_counts)
        for key in proxy._relay_termination_counts:
            proxy._relay_termination_counts[key] = 0

    def tearDown(self) -> None:
        proxy._relay_termination_counts.clear()
        proxy._relay_termination_counts.update(self._saved)

    async def _run(self, reader, writer, timeout=_BODY_END_TIMEOUT) -> str:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(
                    proxy._relay_response_with_status_tap(
                        reader, writer, "POST", "/v1/messages"
                    ),
                    timeout=timeout,
                )
        return buffer.getvalue()

    async def test_body_end_is_counted_and_silent_by_default(self) -> None:
        emitted = await self._run(
            _held_open_reader_of(
                b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi"
            ),
            _CapturingWriter(),
        )
        self.assertEqual(proxy._relay_termination_counts["body-end"], 1)
        # AC7: not one extra log line per request when the gate is off.
        self.assertNotIn("relay-termination", emitted)

    async def test_eof_termination_is_its_own_class(self) -> None:
        await self._run(
            _reader_of(b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\nshort"),
            _CapturingWriter(),
        )
        self.assertEqual(proxy._relay_termination_counts["eof"], 1)
        self.assertEqual(proxy._relay_termination_counts["body-end"], 0)

    async def test_unprovable_framing_is_counted_as_undetermined(self) -> None:
        await self._run(
            _reader_of(b"HTTP/1.1 200 OK\r\n\r\ndata: one\r\n\r\n"),
            _CapturingWriter(),
        )
        self.assertEqual(proxy._relay_termination_counts["undetermined"], 1)

    async def test_debug_gate_emits_the_aggregate_never_an_error(self) -> None:
        before = proxy._EGRESS_DEBUG
        proxy._EGRESS_DEBUG = True
        try:
            emitted = await self._run(
                _held_open_reader_of(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi"
                ),
                _CapturingWriter(),
            )
        finally:
            proxy._EGRESS_DEBUG = before
        lines = _strip_ts(self, emitted.splitlines())
        aggregate = [line for line in lines if "relay-termination" in line]
        self.assertEqual(len(aggregate), 1, lines)
        self.assertIn("body-end=1", aggregate[0])
        self.assertIn("eof=0", aggregate[0])
        self.assertIn("undetermined=0", aggregate[0])
        self.assertIn("DEBUG", aggregate[0])
        self.assertNotIn("ERROR", emitted)


class ChunkedDecoderTests(unittest.TestCase):
    """The decoder alone — the object that replaces a substring search."""

    def _feed(self, *pieces: bytes) -> proxy._ChunkedDecoder:
        decoder = proxy._ChunkedDecoder()
        for piece in pieces:
            decoder.feed(piece)
        return decoder

    def test_terminator_inside_data_is_not_an_end(self) -> None:
        payload = b"0\r\n\r\n"
        decoder = self._feed(
            f"{len(payload):x}\r\n".encode("ascii") + payload + b"\r\n"
        )
        self.assertFalse(decoder.done)
        self.assertFalse(decoder.failed)
        decoder.feed(b"0\r\n\r\n")
        self.assertTrue(decoder.done)

    def test_byte_at_a_time_delivery_decodes_identically(self) -> None:
        body = b"4\r\nabcd\r\n0\r\n\r\n"
        decoder = proxy._ChunkedDecoder()
        for index in range(len(body)):
            self.assertFalse(decoder.done, f"ended early at byte {index}")
            decoder.feed(body[index:index + 1])
        self.assertTrue(decoder.done)

    def test_oversized_size_line_fails_rather_than_buffering(self) -> None:
        decoder = proxy._ChunkedDecoder(cap=64)
        decoder.feed(b"a" * 512)
        self.assertTrue(decoder.failed)
        self.assertFalse(decoder.done)

    def test_a_failed_decoder_stays_failed(self) -> None:
        decoder = self._feed(b"zz\r\n")
        self.assertTrue(decoder.failed)
        decoder.feed(b"0\r\n\r\n")
        self.assertFalse(decoder.done)

    def test_parse_chunk_size_is_strict(self) -> None:
        self.assertEqual(proxy._parse_chunk_size(b"1f"), 31)
        self.assertEqual(proxy._parse_chunk_size(b"0;ext=1"), 0)
        for line in (b"", b" 5", b"5 ", b"0x5", b"-1", b"g"):
            with self.subTest(line=line):
                self.assertIsNone(proxy._parse_chunk_size(line))


class SuiteWiringTests(unittest.TestCase):
    """U11 — the sibling keep-alive suite must actually be executed.

    `scripts/test-pilot-egress-keepalive.py` fences mika#2313's AC1 and was, at
    the time of mika#2317, referenced by no Makefile target and no CI job: it
    was a test that ran nowhere. Found in passing, wired here because this is
    the change that already touches both files.
    """

    _REPO = pathlib.Path(__file__).resolve().parent.parent
    _KEEPALIVE = "scripts/test-pilot-egress-keepalive.py"

    def test_makefile_runs_the_keepalive_suite(self) -> None:
        makefile = (self._REPO / "Makefile").read_text(encoding="utf-8")
        target = makefile.partition("\ntest-pilot-egress-proxy:")[2]
        self.assertTrue(target, "target test-pilot-egress-proxy not found")
        body = target.partition("\n\n")[0]
        self.assertIn(self._KEEPALIVE, body)

    def test_ci_runs_the_keepalive_suite(self) -> None:
        workflow = (
            self._REPO / ".github" / "workflows" / "ci.yml"
        ).read_text(encoding="utf-8")
        job = workflow.partition("\n  pilot-egress-status-tap:")[2]
        self.assertTrue(job, "job pilot-egress-status-tap not found")
        body = job.partition("\n\n")[0]
        self.assertIn(self._KEEPALIVE, body)


class UpstreamOutcomeLoggingTests(unittest.TestCase):
    """The lines an operator greps. These are the product of mika#1901."""

    def _emit(self, status, quota=()) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            proxy._log_upstream_outcome("POST", "/v1/messages?beta=true", status, list(quota))
        return _strip_ts(self, buffer.getvalue().splitlines())

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


class TimestampTests(unittest.TestCase):
    """mika#2030 — every proxy log line leads with one ISO-8601 UTC ms stamp.

    The founding gap: `/var/log/mika/pilot-egress-proxy.log` carried no time
    field, so `grep -c '2026-08-28'` returned 0 and the mika#1772 / #2029
    pilot-stall windows could not be correlated to upstream statuses. These
    tests fence the fix: `_log` is the sole emitter, and it stamps every line.
    """

    def _raw(self, fn, *args) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            fn(*args)
        return buffer.getvalue().splitlines()

    def test_helper_prefixes_a_parseable_iso_utc_millisecond_stamp(self) -> None:
        lines = self._raw(proxy._log, "[egress] ALLOW 127.0.0.1:0")
        self.assertEqual(len(lines), 1)
        match = _TS_PREFIX_RE.match(lines[0])
        self.assertIsNotNone(match, lines[0])
        assert match is not None
        # Parses as a real UTC instant, not just a stamp-shaped string.
        parsed = datetime.datetime.strptime(
            match.group("ts"), "%Y-%m-%dT%H:%M:%S.%fZ"
        )
        self.assertIsNotNone(parsed)
        # The message body is preserved verbatim after the stamp (AC2).
        self.assertEqual(match.group("rest"), "[egress] ALLOW 127.0.0.1:0")

    def test_stamp_answers_the_date_grep_that_returned_zero_before_2030(self) -> None:
        # The ticket's exact reproducer: `grep -c '<today>'` returned 0. A
        # date-prefixed line now matches, so a date grep composes with the
        # sibling logs.
        today = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
        lines = self._raw(proxy._log, "[egress] host-unix listening")
        self.assertTrue(lines[0].startswith(today), lines[0])

    def test_429_line_carries_exactly_one_stamp_at_the_head(self) -> None:
        # The plan's trap (AC3): `_log_upstream_outcome` composes the 429 line
        # in two pieces (`{line} {detail}`). The stamp must land once, at the
        # head, never in the middle of the RATE_LIMITED line, and the quota
        # detail must stay on that same line.
        lines = self._raw(
            proxy._log_upstream_outcome,
            "POST", "/v1/messages?beta=true", 429,
            [("Retry-After", "42"), ("request-id", "req_abc")],
        )
        self.assertEqual(len(lines), 2)
        rate_line = lines[1]
        match = _TS_PREFIX_RE.match(rate_line)
        self.assertIsNotNone(match, rate_line)
        assert match is not None
        rest = match.group("rest")
        # Exactly one stamp: no second stamp embedded in the remainder.
        self.assertIsNone(_TS_PREFIX_RE.match(rest))
        self.assertEqual(
            rest,
            "[anthropic-proxy] RATE_LIMITED POST /v1/messages?beta=true "
            "Retry-After=42 request-id=req_abc",
        )

    def test_429_without_quota_headers_is_one_clean_stamped_line(self) -> None:
        # AC3 anti-vacuity: the same line with no quota detail is still one
        # stamped line, RATE_LIMITED intact, no dangling trailing space.
        lines = self._raw(
            proxy._log_upstream_outcome, "POST", "/v1/messages", 429, [],
        )
        self.assertEqual(len(lines), 2)
        match = _TS_PREFIX_RE.match(lines[1])
        self.assertIsNotNone(match, lines[1])
        assert match is not None
        self.assertEqual(
            match.group("rest"), "[anthropic-proxy] RATE_LIMITED POST /v1/messages"
        )
        self.assertFalse(lines[1].endswith(" "))


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


# ---------------------------------------------------------------------------
# Socket lifecycle in host mode (mika#2041)
#
# A kill does not unlink a unix socket. The orphaned path is what armed the
# launcher's broken guard during the 2026-08-29 incident, so the proxy owes two
# things: clear a stale path on the way in, and clear its own on the way out.
#
# The inbound half already existed (e4f24677 / #1894) and was measured deployed
# during the incident -- these tests lock it rather than reimplement it. The
# outbound half is new.
#
# Driven as real subprocesses: signal delivery and process teardown are the
# behaviour under test, and neither survives being simulated in-process.
# ---------------------------------------------------------------------------


class HostSocketLifecycleTests(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.mkdtemp(prefix="mika-egress-lifecycle-")
        # Never the real /tmp/mika-pilot-egress.sock: a live host proxy owns
        # that path, and a test that unlinks it takes egress down.
        self.sock = os.path.join(self._dir, "egress.sock")
        self._procs: list[subprocess.Popen] = []

    def tearDown(self) -> None:
        for proc in self._procs:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)
        for name in os.listdir(self._dir):
            os.unlink(os.path.join(self._dir, name))
        os.rmdir(self._dir)

    def _spawn_host(self, env: dict | None = None) -> subprocess.Popen:
        child_env = None
        if env is not None:
            child_env = os.environ.copy()
            child_env.update(env)
        proc = subprocess.Popen(
            [sys.executable, str(_PROXY_PATH), "--host-unix", "--socket", self.sock],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            env=child_env,
        )
        self._procs.append(proc)
        return proc

    def _connectable(self) -> bool:
        if not os.path.exists(self.sock):
            return False
        client = socket.socket(socket.AF_UNIX)
        client.settimeout(1)
        try:
            client.connect(self.sock)
            return True
        except OSError:
            return False
        finally:
            client.close()

    def _wait_connectable(self, timeout: float = 10.0) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._connectable():
                return True
            time.sleep(0.05)
        return False

    def _wait_tcp(self, port: int, timeout: float = 10.0) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            probe = socket.socket(socket.AF_INET)
            probe.settimeout(1)
            try:
                probe.connect(("127.0.0.1", port))
                return True
            except OSError:
                time.sleep(0.05)
            finally:
                probe.close()
        return False

    def _make_orphan(self) -> None:
        """Bind then close without unlinking — the shape a kill leaves behind."""
        ghost = socket.socket(socket.AF_UNIX)
        ghost.bind(self.sock)
        ghost.close()
        self.assertTrue(os.path.exists(self.sock))
        self.assertFalse(self._connectable())

    def test_orphaned_socket_is_cleared_before_bind(self) -> None:
        # The regression lock for the behaviour mika#2041's body mistakenly
        # reported as missing. It is not missing; it must not be removed.
        self._make_orphan()
        self._spawn_host()
        self.assertTrue(self._wait_connectable(), "proxy failed to bind over an orphan")

    def test_non_socket_at_path_is_refused_not_unlinked(self) -> None:
        with open(self.sock, "w", encoding="utf-8") as handle:
            handle.write("not a socket")
        proc = self._spawn_host()
        proc.wait(timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        self.assertTrue(os.path.exists(self.sock), "refused path must survive")

    def test_fatal_refusal_diagnostic_is_timestamped(self) -> None:
        # mika#2030: the fatal non-socket refusal reaches pilot-egress-proxy.log
        # via stderr, so it must carry a timestamp like every other line — a
        # bare `sys.exit(msg)` would drop an un-stamped line into a stamped log.
        with open(self.sock, "w", encoding="utf-8") as handle:
            handle.write("not a socket")
        proc = self._spawn_host()
        _, stderr = proc.communicate(timeout=10)
        self.assertNotEqual(proc.returncode, 0)
        text = stderr.decode()
        lines = [line for line in text.splitlines() if line.strip()]
        self.assertTrue(lines, "expected a fatal diagnostic on stderr")
        for line in lines:
            match = _TS_PREFIX_RE.match(line)
            self.assertIsNotNone(match, f"fatal line not timestamped: {line!r}")
            assert match is not None
            datetime.datetime.strptime(match.group("ts"), "%Y-%m-%dT%H:%M:%S.%fZ")
        self.assertIn("refusing to unlink non-socket", text)

    def test_sigterm_unlinks_the_socket(self) -> None:
        proc = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        proc.send_signal(signal.SIGTERM)
        proc.wait(timeout=10)
        self.assertFalse(
            os.path.exists(self.sock),
            "SIGTERM left the socket behind — the orphan that arms the launcher guard",
        )

    def test_sigterm_terminates_promptly_with_a_live_client(self) -> None:
        # The regression that the first shape of this fix introduced: awaiting
        # the server's close (via `async with server` / `wait_closed()`) blocks
        # on every live client handler. An idle client holds one for the
        # 10s header-read timeout; a real CONNECT tunnel holds one for the
        # whole pilot session. Either way the operator's `kill` appears to do
        # nothing and the next move is `kill -9` -- which orphans the socket
        # and re-creates the class this file exists to close.
        #
        # An idle client is used deliberately: it needs no network, so the
        # bound is deterministic in CI, and it still reproduces the hang.
        proc = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        client = socket.socket(socket.AF_UNIX)
        client.connect(self.sock)
        try:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=4)
        except subprocess.TimeoutExpired:
            self.fail(
                "SIGTERM did not terminate the proxy within 4s while a client "
                "was connected -- shutdown is waiting on live handlers"
            )
        finally:
            client.close()
        self.assertFalse(os.path.exists(self.sock))

    def test_sigint_unlinks_the_socket(self) -> None:
        proc = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        proc.send_signal(signal.SIGINT)
        proc.wait(timeout=10)
        self.assertFalse(os.path.exists(self.sock))

    def test_shutdown_does_not_unlink_a_successors_socket(self) -> None:
        # The property the inode comparison buys, and the reason an
        # `is_socket()` check is not enough: it is equally true of a DIFFERENT
        # proxy's live socket. A launcher that starts a replacement while the
        # old one is still winding down must not end up with a running proxy
        # whose path was deleted underneath it -- that state is invisible to
        # every liveness probe and makes the launcher spawn a new proxy on
        # every subsequent dispatch.
        first = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        first_ino = os.stat(self.sock).st_ino

        second = self._spawn_host()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            try:
                if os.stat(self.sock).st_ino != first_ino:
                    break
            except FileNotFoundError:
                pass
            time.sleep(0.05)
        else:
            self.fail("successor proxy never took over the path")
        self.assertTrue(self._wait_connectable())

        first.send_signal(signal.SIGTERM)
        first.wait(timeout=10)
        self.assertTrue(
            self._connectable(),
            "the retiring proxy deleted its successor's live socket",
        )
        self.assertIsNone(second.poll())

    def test_startup_emits_a_begin_breadcrumb_before_bind(self) -> None:
        # mika#2051: the 2026-08-29 incident left 1569 log lines with nothing
        # from the two dead proxies. The proxy must announce its own arrival so
        # a future pre-bind death is bounded from below: if even this line is
        # missing, the process died before Python ran (exec failure); if it is
        # present but `listening` is not, the death is inside the pre-bind
        # window. The line is emitted before bind, so it is here the moment the
        # socket is connectable.
        proc = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        line = proc.stderr.readline().decode().rstrip("\n")
        # The breadcrumb is an operator-log line, so it must carry mika#2030's
        # timestamp like every other; strip it before the message assertions.
        rest = _strip_ts(self, [line])[0]
        self.assertIn("pilot_egress_startup.begin", rest)
        self.assertIn(f"pid={proc.pid}", rest)

    def test_pre_bind_signal_names_its_cause(self) -> None:
        # The core regression lock for mika#2051. A SIGTERM landing in the
        # pre-bind window used to kill the proxy mutely: the graceful handlers
        # are armed only after bind. Park the proxy deterministically inside
        # that window and signal it -- the death must now NAME its cause rather
        # than vanish. This is the trace the incident ticket was filed to
        # receive.
        proc = self._spawn_host(env={"_MIKA_EGRESS_PREBIND_TEST_BARRIER": "5"})
        # The `.begin` breadcrumb proves we are past the early handler install
        # and parked before bind -- exactly where the incident struck.
        begin = proc.stderr.readline().decode().rstrip("\n")
        self.assertIn("pilot_egress_startup.begin", _strip_ts(self, [begin])[0])
        self.assertFalse(
            self._connectable(), "proxy bound despite the pre-bind barrier"
        )
        proc.send_signal(signal.SIGTERM)
        _, stderr = proc.communicate(timeout=10)
        self.assertEqual(
            proc.returncode, 3, "a signalled-before-bind exit must be distinguishable"
        )
        # Every emitted line stays timestamped (mika#2030) through the abort
        # path too; strip-and-assert as the shared helpers do.
        lines = _strip_ts(self, stderr.decode().splitlines())
        text = "\n".join(lines)
        self.assertIn("pilot_egress_startup.signalled", text)
        self.assertIn("SIGTERM", text)
        self.assertNotIn("host-unix listening", text)
        self.assertFalse(
            os.path.exists(self.sock),
            "an aborted startup must not leave a socket behind",
        )

    def test_sandbox_mode_never_unlinks_the_unix_socket(self) -> None:
        # The sandbox side connects to the socket; it does not own it. A shutdown
        # there must leave the host's socket standing.
        host = self._spawn_host()
        self.assertTrue(self._wait_connectable())
        with socket.socket(socket.AF_INET) as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        shim = subprocess.Popen(
            [
                sys.executable,
                str(_PROXY_PATH),
                "--sandbox-tcp",
                str(port),
                "--socket",
                self.sock,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._procs.append(shim)
        # The port was picked by binding and releasing, so another process can
        # claim it in between and the shim dies at bind time. Assert it is
        # actually listening before signalling it -- otherwise a shim that
        # never started satisfies both assertions below and the test proves
        # nothing.
        self.assertTrue(
            self._wait_tcp(port), f"sandbox shim never came up on port {port}"
        )
        shim.send_signal(signal.SIGTERM)
        shim.wait(timeout=10)
        self.assertTrue(
            self._connectable(), "sandbox shutdown took the host socket with it"
        )
        self.assertIsNone(host.poll(), "host proxy should still be running")


# ---------------------------------------------------------------------------
# Readiness-probe vs. genuine error (mika#2042)
#
# dispatch-lib.sh's `_pilot_egress_sock_connectable` (and the sandbox's bounded
# --sandbox-tcp wait loop) probe the proxy before every dispatch with a bare
# connect + close: no CONNECT line is ever sent. The proxy answered that
# already-gone peer with a 400, the write's drain raised `Connection lost`, and
# the `except Exception` sink logged `[egress] ERROR unknown: Connection lost`.
# 1045 of 2747 [egress] lines were this one benign non-event, and they were
# read as a broken transport during mika#2029. These tests fence both halves of
# the fix: the probe stops crying ERROR, and a peer that DID speak still does.
# ---------------------------------------------------------------------------


class ReadinessProbeVsErrorTests(unittest.IsolatedAsyncioTestCase):
    class _Writer:
        """StreamWriter stand-in. `fail_drain` reproduces a peer that has
        already closed: the 400 write lands, the drain that follows raises."""

        def __init__(self, fail_drain: bool = False) -> None:
            self.chunks: list[bytes] = []
            self._fail_drain = fail_drain
            self.closed = False

        def write(self, data: bytes) -> None:
            self.chunks.append(bytes(data))

        async def drain(self) -> None:
            if self._fail_drain:
                raise ConnectionResetError("Connection lost")

        def close(self) -> None:
            self.closed = True

    async def _run(self, reader, writer) -> list[str]:
        buffer = io.StringIO()
        with contextlib.redirect_stderr(buffer):
            await proxy.handle_host_client(reader, writer)
        # mika#2030: strip (and assert) the ISO-8601 stamp `_log` prepends, so
        # these `startswith("[egress] ...")` classifications read against the
        # message body — and AC1 (every line stamped) is enforced here too.
        return _strip_ts(self, buffer.getvalue().splitlines())

    async def test_connect_then_close_emits_no_error(self) -> None:
        # The exact 1045-line shape: EOF before a single request byte, and a
        # peer already gone so even the 400 cannot be delivered.
        before = proxy._readiness_probe_count
        lines = await self._run(_reader_of(), self._Writer(fail_drain=True))
        self.assertEqual(lines, [], f"benign probe must be silent, got {lines}")
        # Silenced, but not swallowed: it is counted, so the fix is not merely
        # taping over the log. (Anti-vacuity, counter half.)
        self.assertEqual(proxy._readiness_probe_count, before + 1)

    async def test_malformed_connect_still_emits_one_error(self) -> None:
        # A peer that DID speak — a malformed CONNECT line — then dropped. A
        # genuine client/transport error: it must keep its single ERROR line,
        # or the fix would just be silencing everything. (Anti-vacuity.)
        before = proxy._readiness_probe_count
        lines = await self._run(
            _reader_of(b"CONNECT nonsense\r\n\r\n"),
            self._Writer(fail_drain=True),
        )
        errors = [line for line in lines if line.startswith("[egress] ERROR")]
        self.assertEqual(len(errors), 1, f"expected one ERROR, got {lines}")
        # And it was NOT miscounted as a readiness probe.
        self.assertEqual(proxy._readiness_probe_count, before)

    async def test_truncated_request_after_bytes_is_not_the_probe(self) -> None:
        # Bytes arrived and then the stream truncated before the blank line:
        # the peer spoke, so this is not the no-CONNECT probe. It must not be
        # counted as one; a reset while answering still surfaces as ERROR.
        before = proxy._readiness_probe_count
        lines = await self._run(
            _reader_of(b"CONNECT api.github.com:443\r\nHost: x"),
            self._Writer(fail_drain=True),
        )
        self.assertEqual(proxy._readiness_probe_count, before)
        self.assertTrue(
            any(line.startswith("[egress] ERROR") for line in lines), lines
        )

    async def test_probe_is_surfaced_as_debug_never_error(self) -> None:
        # With the debug gate on, the same benign event is visible as a DEBUG
        # aggregate carrying the running count — never as ERROR.
        before_count = proxy._readiness_probe_count
        before_flag = proxy._EGRESS_DEBUG
        proxy._EGRESS_DEBUG = True
        try:
            lines = await self._run(_reader_of(), self._Writer())
        finally:
            proxy._EGRESS_DEBUG = before_flag
        joined = "\n".join(lines)
        self.assertIn("[egress] DEBUG readiness-probe", joined)
        self.assertNotIn("ERROR", joined)
        self.assertEqual(proxy._readiness_probe_count, before_count + 1)


if __name__ == "__main__":
    unittest.main(verbosity=2, argv=[sys.argv[0]])
