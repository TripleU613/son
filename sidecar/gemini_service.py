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
import logging
import os
import tempfile

from fastapi import FastAPI, File, UploadFile
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


def _accounts() -> list[tuple[str, str]]:
    """Cookie pairs, from `GEMINI_COOKIES` as `psid:psidts,psid:psidts,...`.

    Several accounts are supported because one account's quota is not enough for
    a gallery's worth of uploads. Nothing here manages, validates, or reasons
    about the accounts themselves -- it just takes the next one in the ring.
    """
    raw = os.environ.get("GEMINI_COOKIES", "").strip()
    pairs = []
    for chunk in raw.split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        psid, _, psidts = chunk.partition(":")
        if psid and psidts:
            pairs.append((psid.strip(), psidts.strip()))
    return pairs


class Pool:
    """Round-robin over initialised clients, with a lock per client.

    A GeminiClient holds one conversation-bearing HTTP session, so two requests
    sharing one concurrently interleave their state. The lock makes each client
    serial while still letting N accounts work in parallel.
    """

    def __init__(self) -> None:
        self.clients: list[tuple[GeminiClient, asyncio.Lock]] = []
        self._ring = itertools.cycle([0])

    async def start(self) -> None:
        for i, (psid, psidts) in enumerate(_accounts()):
            client = GeminiClient(psid, psidts)
            try:
                # auto_refresh keeps __Secure-1PSIDTS current in the background;
                # it rotates roughly every half hour and a stale one fails every
                # call. The refreshed value is cached under /tmp (tmpfs), so a
                # container restart re-derives it from the seed cookie.
                await client.init(timeout=GEMINI_TIMEOUT, auto_refresh=True)
            except Exception as e:  # noqa: BLE001 - one bad account must not stop the rest
                log.error("account %d failed to initialise: %s", i, e)
                continue
            self.clients.append((client, asyncio.Lock()))
            log.info("account %d ready", i)

        if not self.clients:
            log.warning("no usable Gemini accounts; every request will return 502")
        self._ring = itertools.cycle(range(len(self.clients) or 1))

    def next(self) -> tuple[GeminiClient, asyncio.Lock] | None:
        if not self.clients:
            return None
        return self.clients[next(self._ring)]


pool = Pool()
app = FastAPI(title="gemini sidecar")


@app.on_event("startup")
async def _startup() -> None:
    await pool.start()


@app.get("/health")
async def health() -> dict:
    return {"accounts": len(pool.clients)}


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
    client, lock = entry
    data = await image.read()
    fd, src = tempfile.mkstemp(suffix=".png", dir="/tmp")
    os.close(fd)
    with open(src, "wb") as f:
        f.write(data)
    return client, lock, src


@app.post("/judge")
async def judge(image: UploadFile = File(...)) -> Response:
    """Is it safe, and is it a son? Two words back, no JSON schema asked of the
    model -- there are two questions with two possible answers each."""
    got = await _with_temp(image)
    if got is None:
        return JSONResponse({"reason": "no Gemini account available"}, status_code=502)
    client, lock, src = got
    try:
        async with lock:
            judged = await client.generate_content(
                JUDGE_PROMPT, files=[src], model=JUDGE_MODEL, temporary=True
            )
    except Exception as e:  # noqa: BLE001
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
    client, lock, src = got
    try:
        return await _square(client, lock, src)
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
