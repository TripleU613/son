"""Gemini sidecar: judge an upload, then hand back a square version of it.

Why this exists as a separate process at all: the Gemini web client
(HanaokaYuzu/Gemini-API, pip `gemini_webapi`) is Python, and it drives
gemini.google.com with browser cookies rather than an API key. The rest of the
app is Rust and has no way to speak that protocol, so this is the smallest
possible bridge -- one endpoint, no state, no database, reachable only from the
app container on the private compose network.

Two endpoints, one Gemini call each:

  POST /judge   -> 200 {"verdict": "PASS"|"FAIL", "topic": "SON"|"NOTSON"}
                   502 {"reason": "..."}
  POST /square  -> 200 image bytes
                   502 {"reason": "..."}

Two calls per upload rather than one prompt doing both, because asking a single
prompt to judge *and* generate makes the model answer as text and refuse the image
("I cannot generate, edit, or modify images") -- observed directly, not guessed.
Judging first also means an unsafe upload never costs an image generation.

They are separate *endpoints* rather than one /process doing both so the caller
can report which phase it is in. Judging takes a few seconds and generating takes
most of a minute; behind one endpoint the whole wait had to be labelled "scanning",
which is a progress bar that lies about what it is waiting for.

The reject/accept decision lives in the caller, not here: this reports what Gemini
said, and `crate::gemini` decides what that means.
"""

from __future__ import annotations

import asyncio
import itertools
import hmac
import logging
import os
import struct
import tempfile
import zlib

from fastapi import FastAPI, File, Header, UploadFile
from fastapi.responses import JSONResponse, Response
from gemini_webapi import GeminiClient
from gemini_webapi.constants import Model

log = logging.getLogger("gemini-sidecar")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

# Two lines, plain words, no JSON. Asked for exactly because a schema buys
# nothing here: there are two questions and each has two possible answers.
JUDGE_PROMPT = """Answer in exactly two lines and nothing else.
Line 1: PASS or FAIL. FAIL if the image is sexual, nude, gore, hateful, or
otherwise unsafe to publish on a public website. PASS otherwise.
Line 2: SON or NOTSON. SON if this is the "son" meme or any wordplay variant of
it -- a crying-laughing face, or a thing whose name hides the word "son"
(Sonion, Capri-Son, Dy-Son, Sonflower). NOTSON if it is unrelated."""

# A pure image request. Any judging language in here and the model replies with
# prose instead of a picture.
SQUARE_PROMPT = """Recreate this exact image as a perfect 1:1 square. Keep the
same subject, colours and style. Fill the entire square edge to edge -- no
borders, no letterboxing, no padding bars, no added text, no caption. Clean it
up: sharp edges, no compression artefacts."""

# Judging is a cheap classification; generating is not. Flash reads images fine
# and is faster, Pro is the one that reliably returns an image.
JUDGE_MODEL = Model.BASIC_FLASH
IMAGE_MODEL = Model.BASIC_PRO

GEMINI_TIMEOUT = int(os.environ.get("GEMINI_TIMEOUT", "120"))

# How often to poke Gemini to keep the session from going stale on its own.
# __Secure-1PSIDTS rotates roughly every half hour and gemini_webapi refreshes it
# in the background, but that refresh is what keeps the session alive -- a sidecar
# that sits idle overnight comes back to an expired session. 15 minutes is
# comfortably inside the rotation window.
KEEPALIVE_SECONDS = int(os.environ.get("GEMINI_KEEPALIVE_SECONDS", "900"))


# Where a runtime cookie update is remembered, so restarting the *process* (not
# the container) keeps it. /tmp is the only writable path and is a tmpfs, which
# is the right trade: cookies are credentials and should not outlive the
# container, while GEMINI_COOKIES stays the seed a fresh container starts from.
COOKIE_CACHE = "/tmp/cookies.txt"

# Shared with the app, which is the only thing that should be able to swap
# credentials. The sidecar publishes no port and is only reachable on the private
# compose network, so this is the second lock rather than the first -- it exists
# so that a future compose mistake exposing the port is not immediately a
# credential-swap endpoint open to the world.
SIDECAR_KEY = os.environ.get("SIDECAR_KEY", "")

# A 1x1 PNG on disk, used to prove an account works.
#
# It has to be an *image* prompt, and that is the whole point. With dead cookies
# gemini_webapi logs `Account status: UNAUTHENTICATED` as a warning and carries on
# unauthenticated, where a plain text prompt still answers -- so a text probe
# reported healthy while every real call failed with error 1100. Uploading an
# image is the capability this service actually depends on, so it is the only
# thing worth checking. A path rather than BytesIO because the uploader needs a
# filename it has no way to accept for in-memory data.
PROBE_PNG = "/tmp/probe.png"


def _probe_png() -> bytes:
    """A valid 2x2 PNG, built rather than pasted as a hex blob.

    Constructed with struct+zlib so it is obviously correct and needs no image
    library in this container. 2x2 rather than 1x1 because some decoders treat a
    single pixel as degenerate, and two rows costs nothing.
    """
    w = h = 2
    raw = b"".join(b"\x00" + b"\x80\x80\x80\xff" * w for _ in range(h))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def _write_probe() -> None:
    try:
        with open(PROBE_PNG, "wb") as f:
            f.write(_probe_png())
    except OSError as e:  # noqa: BLE001
        log.error("could not write the probe image (%s); checks will be skipped", e)


def _parse(raw: str) -> list[tuple[str, str]]:
    """`psid:psidts,psid:psidts,...` -> pairs. Malformed entries are dropped
    rather than raising: one bad paste should not take down the ones that work."""
    pairs = []
    for chunk in raw.split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        psid, _, psidts = chunk.partition(":")
        if psid and psidts:
            pairs.append((psid.strip(), psidts.strip()))
    return pairs


def _accounts() -> list[tuple[str, str]]:
    """The cookies to use: a runtime update if one has been made, else the env.

    Several accounts are supported because one account's quota is not enough for
    a gallery's worth of uploads. Nothing here manages, validates, or reasons
    about the accounts themselves -- it just takes the next one in the ring.
    """
    try:
        with open(COOKIE_CACHE) as f:
            cached = _parse(f.read())
        if cached:
            log.info("using %d cookie pair(s) from a runtime update", len(cached))
            return cached
    except OSError:
        pass
    return _parse(os.environ.get("GEMINI_COOKIES", "").strip())


# How many consecutive failures make an account count as unusable in /health.
# More than one, because a single 1100 is often transient; small, because the
# whole point is to notice quickly.
UNHEALTHY_AFTER = 3


class Pool:
    """Round-robin over initialised clients, with a lock per client.

    A GeminiClient holds one conversation-bearing HTTP session, so two requests
    sharing one concurrently interleave their state. The lock makes each client
    serial while still letting N accounts work in parallel.

    Tracks consecutive failures per client, because `init()` succeeding proves
    almost nothing: with expired cookies it still reports "Gemini client
    initialized successfully", logs `Account status: UNAUTHENTICATED` as a
    warning, and then fails every single call with error 1100. A healthcheck
    counting initialised clients therefore said "healthy" through a total
    outage -- observed in production, which is what prompted this.
    """

    def __init__(self) -> None:
        self.clients: list[tuple[GeminiClient, asyncio.Lock]] = []
        self.failures: list[int] = []
        self._ring = itertools.cycle([0])

    async def start(self) -> None:
        await self._build(_accounts())

    async def replace(self, pairs: list[tuple[str, str]]) -> int:
        """Swap in new cookies and report how many actually work.

        Builds and *proves* the new clients before replacing the live list, so a
        paste of dead cookies leaves the working ones in place rather than taking
        screening down in exchange for nothing.
        """
        before = self.clients, self.failures
        self.clients, self.failures = [], []
        await self._build(pairs)
        if not self.clients:
            self.clients, self.failures = before
            return 0
        self._ring = itertools.cycle(range(len(self.clients)))
        return len(self.clients)

    async def _build(self, pairs: list[tuple[str, str]]) -> None:
        for i, (psid, psidts) in enumerate(pairs):
            client = GeminiClient(psid, psidts)
            try:
                # auto_refresh keeps __Secure-1PSIDTS current in the background;
                # it rotates roughly every half hour and a stale one fails every
                # call. The refreshed value is cached under /tmp (tmpfs), so a
                # container restart re-derives it from the seed cookie.
                await client.init(timeout=GEMINI_TIMEOUT, auto_refresh=True)
                # init() is not a validity check: it succeeds for expired cookies
                # and for outright garbage ("garbage:alsogarbage" was accepted,
                # then failed every real call). Nor is a text prompt -- those still
                # answer unauthenticated. Only an image prompt proves the account,
                # which is exactly what /judge and /square need.
                await client.generate_content(
                    "Reply with the single word: ok",
                    files=[PROBE_PNG],
                    model=JUDGE_MODEL,
                    temporary=True,
                )
            except Exception as e:  # noqa: BLE001 - one bad account must not stop the rest
                log.error("account %d is not usable: %s", i, str(e)[:160])
                continue
            self.clients.append((client, asyncio.Lock()))
            self.failures.append(0)
            log.info("account %d ready", i)

        if not self.clients:
            log.warning("no usable Gemini accounts; every request will return 502")
        self._ring = itertools.cycle(range(len(self.clients) or 1))

    def next(self) -> tuple[int, GeminiClient, asyncio.Lock] | None:
        if not self.clients:
            return None
        i = next(self._ring)
        client, lock = self.clients[i]
        return i, client, lock

    def succeeded(self, i: int) -> None:
        if i < len(self.failures):
            self.failures[i] = 0

    def failed(self, i: int) -> None:
        if i >= len(self.failures):
            return
        self.failures[i] += 1
        if self.failures[i] == UNHEALTHY_AFTER:
            log.error(
                "account %d has failed %d calls in a row; reporting unhealthy",
                i,
                UNHEALTHY_AFTER,
            )

    def usable(self) -> int:
        return sum(1 for f in self.failures if f < UNHEALTHY_AFTER)


def _authorised(key: str | None) -> bool:
    """Constant-time compare, and an unset SIDECAR_KEY refuses everything rather
    than allowing everything -- an empty shared secret is a misconfiguration, not
    a decision to run open."""
    return bool(SIDECAR_KEY) and bool(key) and hmac.compare_digest(key, SIDECAR_KEY)


pool = Pool()
app = FastAPI(title="gemini sidecar")


async def _keepalive() -> None:
    """Ask each account something trivial, forever, so the session stays live.

    A one-word text prompt, no image: the point is to exercise the session and let
    the library's cookie refresh keep up, not to do work. Failures are counted the
    same way a real call's are, so an expired session shows up in /health within
    minutes instead of at the next upload.
    """
    while True:
        await asyncio.sleep(KEEPALIVE_SECONDS)
        for i, (client, lock) in enumerate(list(pool.clients)):
            try:
                async with lock:
                    # With the probe image, for the same reason _build uses it: a
                    # text-only keepalive would keep reporting success against an
                    # unauthenticated session.
                    await client.generate_content(
                        "Reply with the single word: ok",
                        files=[PROBE_PNG],
                        model=JUDGE_MODEL,
                        temporary=True,
                    )
                pool.succeeded(i)
                log.debug("keepalive ok for account %d", i)
            except Exception as e:  # noqa: BLE001
                pool.failed(i)
                log.warning("keepalive failed for account %d: %s", i, e)


@app.on_event("startup")
async def _startup() -> None:
    _write_probe()
    await pool.start()
    # Held so it is not garbage collected mid-flight; never awaited, since it
    # runs for the life of the process.
    app.state.keepalive = asyncio.create_task(_keepalive())


@app.get("/health")
async def health() -> Response:
    """`accounts` is what the container healthcheck reads, and it counts accounts
    that are actually answering -- not accounts that merely started up. 503 as
    well, so anything checking status rather than the body also sees it."""
    body = {
        "accounts": pool.usable(),
        "initialised": len(pool.clients),
        "consecutive_failures": pool.failures,
    }
    return JSONResponse(body, status_code=200 if pool.usable() else 503)


async def _with_temp(image: UploadFile):
    """Spool the upload to a real path and hand back (client, lock, path).

    A real path on disk, not BytesIO: gemini_webapi's uploader requires an
    explicit filename for in-memory data and `generate_content` gives no way to
    supply one, so a BytesIO silently uploads nothing and the model then answers
    in prose with no image attached. Observed exactly that before switching.
    """
    entry = pool.next()
    if entry is None:
        return None
    idx, client, lock = entry
    data = await image.read()
    fd, src = tempfile.mkstemp(suffix=".png", dir="/tmp")
    os.close(fd)
    with open(src, "wb") as f:
        f.write(data)
    return idx, client, lock, src


@app.post("/cookies")
async def cookies(
    payload: dict,
    x_sidecar_key: str | None = Header(default=None),
) -> Response:
    """Swap in fresh cookies without a redeploy.

    This exists because the cookies expire and there is no way around that: the
    Gemini web client authenticates as a browser session, and a session dies. What
    can be fixed is the cost of replacing them -- editing a GitHub secret and
    waiting out a ~12 minute CI deploy, versus pasting two values into /admin and
    having screening back in seconds.

    Verified before being accepted: the new cookies have to actually initialise, or
    the request is refused and whatever was working stays working.
    """
    if not _authorised(x_sidecar_key):
        return JSONResponse({"reason": "unauthorised"}, status_code=403)

    pairs = _parse(str(payload.get("cookies", "")))
    if not pairs:
        return JSONResponse(
            {"reason": "expected psid:psidts, comma-separated for several accounts"},
            status_code=400,
        )

    count = await pool.replace(pairs)
    if not count:
        return JSONResponse(
            {"reason": "none of those cookies could authenticate; kept the previous ones"},
            status_code=400,
        )

    # Remembered so a process restart inside the container keeps them.
    try:
        with open(COOKIE_CACHE, "w") as f:
            f.write(",".join(f"{p}:{t}" for p, t in pairs))
    except OSError as e:  # noqa: BLE001
        log.warning("could not cache cookies (%s); they are live but not persisted", e)

    log.info("cookies replaced at runtime; %d account(s) ready", count)
    return JSONResponse({"accounts": count})


@app.post("/judge")
async def judge(image: UploadFile = File(...)) -> Response:
    """Is it safe, and is it a son? Two words back, no JSON schema asked of the
    model -- there are two questions with two possible answers each."""
    got = await _with_temp(image)
    if got is None:
        return JSONResponse({"reason": "no Gemini account available"}, status_code=502)
    idx, client, lock, src = got
    try:
        async with lock:
            judged = await client.generate_content(
                JUDGE_PROMPT, files=[src], model=JUDGE_MODEL, temporary=True
            )
        pool.succeeded(idx)
    except Exception as e:  # noqa: BLE001
        pool.failed(idx)
        log.error("judge call failed: %s", e)
        return JSONResponse({"reason": f"gemini judge failed: {e}"}, status_code=502)
    finally:
        try:
            os.unlink(src)
        except OSError:
            pass

    lines = [l.strip().upper() for l in judged.text.splitlines() if l.strip()]
    verdict = lines[0] if lines else ""
    topic = lines[1] if len(lines) > 1 else ""
    log.info("verdict=%r topic=%r", verdict, topic)
    # Reported as-is, including an empty answer. The caller fails closed on
    # anything that is not an explicit PASS.
    return JSONResponse({"verdict": verdict, "topic": topic})


@app.post("/square")
async def square(image: UploadFile = File(...)) -> Response:
    """The slow one: Gemini redrawing the image as a 1:1 square."""
    got = await _with_temp(image)
    if got is None:
        return JSONResponse({"reason": "no Gemini account available"}, status_code=502)
    idx, client, lock, src = got
    try:
        resp = await _square(client, lock, src)
        pool.succeeded(idx) if resp.status_code == 200 else pool.failed(idx)
        return resp
    finally:
        try:
            os.unlink(src)
        except OSError:
            pass

    # A real path on disk, not BytesIO: gemini_webapi's uploader requires an
    # explicit filename for in-memory data and `generate_content` gives no way to
    # supply one, so a BytesIO silently uploads nothing and the model then answers
    # in prose with no image attached. Observed exactly that before switching.
    # /tmp is tmpfs and the file is removed in the finally below.
    fd, src = tempfile.mkstemp(suffix=".png", dir="/tmp")
    os.close(fd)
    with open(src, "wb") as f:
        f.write(data)

    try:
        return await _process(client, lock, src)
    finally:
        try:
            os.unlink(src)
        except OSError:
            pass


async def _square(client: GeminiClient, lock: asyncio.Lock, src: str) -> Response:
    async with lock:
        try:
            regen = await client.generate_content(
                SQUARE_PROMPT, files=[src], model=IMAGE_MODEL, temporary=True
            )
        except Exception as e:  # noqa: BLE001
            log.error("image call failed: %s", e)
            return JSONResponse({"reason": f"gemini image failed: {e}"}, status_code=502)

        if not regen.images:
            # The model answered in prose instead of producing a picture. The
            # caller keeps the original rather than losing the upload.
            return JSONResponse({"reason": "gemini returned no image"}, status_code=502)

        # `save()` is the only way to get the bytes -- the Image type exposes no
        # reader -- so it round-trips through tmpfs and is deleted immediately.
        path = None
        try:
            path = await regen.images[0].save(
                path="/tmp", filename=f"regen-{os.getpid()}-{id(regen)}.bin", verbose=False
            )
            with open(path, "rb") as f:
                out = f.read()
        finally:
            if path:
                try:
                    os.unlink(path)
                except OSError:
                    pass

    # Whatever Gemini's container was (JPEG today), the app re-encodes to PNG on
    # the way to storage, so the format here is not load-bearing.
    return Response(content=out, media_type="application/octet-stream")
