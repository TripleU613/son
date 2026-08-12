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

Several calls run at once (see CONCURRENCY): one account is opened as several
independent sessions, which is only safe because this process no longer refreshes
cookies -- the keeper container does. Details at CONCURRENCY.

The reject/accept decision lives in the caller, not here: this reports what Gemini
said, and `crate::gemini` decides what that means.
"""

from __future__ import annotations

import asyncio
import hmac
import logging
import os
import struct
import tempfile
import uuid
import zlib
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

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

# An *edit*, stated as one over and over, because the previous version of this
# prompt asked it to "recreate this exact image ... keep the same subject" and
# Gemini took that as licence to redraw: a photograph of a person came back as a
# different person's face. Anything that can be read as "produce a picture like
# this one" produces a new picture, so the words "recreate", "generate" and
# "clean it up" are deliberately gone -- that last one invited repainting too.
#
# The subject is stated as untouchable first and the permitted changes are then
# enumerated, because a vague instruction is treated as permission to
# reinterpret. No judging language: any of that and the model replies with prose
# instead of a picture.
SQUARE_PROMPT = """Edit the image I gave you. Do not generate a new image, do not
redraw it, do not reinterpret it.

Keep the subject exactly as it is, pixel for pixel wherever you can: the same
people, the same faces, the same animals, objects, pose, expression, lighting,
colours and art style. Never substitute a different person or a different face,
never change anyone's identity or appearance, never restyle, re-illustrate,
repaint, beautify or "improve" anything, and never add anything that was not
already in the picture.

Change only these things:
1. Remove text that sits on top of the picture: captions, subtitles, watermarks,
   logos, stickers, usernames, meme text. Fill the space it leaves with the
   background that belongs there. Text that is part of the scene itself -- a
   street sign, a label on a bottle, print on a T-shirt -- stays exactly as it is.
2. Remove borders, frames, outlines, rounded corners, drop shadows and letterbox
   or pillarbox bars, so the picture itself reaches every edge.
3. Make the result a 1:1 square, filled edge to edge. Either crop, or extend the
   existing background outwards -- whichever keeps the whole subject in view.
   Never stretch, squash or distort, and never pad with a flat colour.

Return the edited image and nothing else."""

# Deliberately trivial, and used both to prove an account and to keep it warm --
# what is being tested is that an image reaches Gemini at all, not the answer.
PROBE_PROMPT = "Reply with the single word: ok"

# Judging is a cheap classification; generating is not. Flash reads images fine
# and is faster, Pro is the one that reliably returns an image.
JUDGE_MODEL = Model.BASIC_FLASH
IMAGE_MODEL = Model.BASIC_PRO

def _int_env(name: str, default: int) -> int:
    """An int from the environment, falling back rather than refusing to start.

    Because compose spells an optional variable `${FOO:-}`, which hands over an
    empty *string* when the host has not set it -- and a bare int("") is an
    exception at import time, i.e. a container that crash-loops and takes
    screening down over a variable nobody set on purpose.
    """
    raw = os.environ.get(name, "").strip()
    try:
        return int(raw)
    except ValueError:
        if raw:
            log.warning("%s=%r is not a number; using %d", name, raw, default)
        return default


GEMINI_TIMEOUT = _int_env("GEMINI_TIMEOUT", 120)

# How many uploads may be in flight at once.
#
# This used to be one, effectively: the pool held a single client per cookie pair
# behind a lock, so a /square blocked every other upload for its full 30-80
# seconds. It had to be one because *two clients on one account fight over the
# cookies*: gemini_webapi refreshes the rotating __Secure-1PSIDTS in the
# background, and two clients refreshing the same account invalidate each other's
# copy -- which took screening down earlier in this project.
#
# That constraint is gone because the refreshing moved out. The keeper container
# owns it now: it reads a real signed-in browser's cookie jar and POSTs the
# current pair to /cookies (see keeper/session_keeper.py), so clients here are
# built with auto_refresh=False, have nothing to rotate, and cannot invalidate
# one another. Several sessions on one cookie pair are then just several sockets.
#
# A total across all accounts rather than a number per account: the binding
# constraint is this container (512MB, half a core), not any one account's quota,
# and the round-robin spreads the calls over the accounts either way.
CONCURRENCY = max(1, _int_env("GEMINI_CONCURRENCY", 5))

# How often to poke Gemini to keep the session from going stale on its own.
# __Secure-1PSIDTS rotates roughly every half hour and the keeper pushes the fresh
# value, but a session nothing uses still lapses -- a sidecar that sits idle
# overnight comes back dead. 15 minutes is comfortably inside the rotation window.
KEEPALIVE_SECONDS = _int_env("GEMINI_KEEPALIVE_SECONDS", 900)


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


def _forget_cached_cookies() -> None:
    """Drop gemini_webapi's own cookie cache before opening any client.

    The library caches cookies per __Secure-1PSID under /tmp and *prefers that
    cache over the values a client is constructed with*. That was correct while
    the library did the rotating. Now the keeper does, so the freshest cookies in
    existence are the ones being handed in, and a leftover cache file only means
    the first value tried is one Google has already retired. Nothing is lost by
    forgetting it: the pair the keeper pushed is remembered in COOKIE_CACHE.
    """
    root = os.environ.get("GEMINI_COOKIE_PATH")
    cache = Path(root) if root else Path(tempfile.gettempdir()) / "gemini_webapi"
    for stale in cache.glob(".cached_cookies_*.json"):
        try:
            stale.unlink()
        except OSError as e:  # noqa: BLE001 - best effort; a leftover is not fatal
            log.warning("could not clear %s (%s)", stale, e)


# How many consecutive failures make an account count as unusable in /health.
# More than one, because a single 1100 is often transient; small, because the
# whole point is to notice quickly.
UNHEALTHY_AFTER = 3


class Account:
    """One cookie pair, and how it is currently behaving.

    Failures are counted per account rather than per session, even though there
    are now several sessions per account: every session presents the same
    cookies, so when those expire all of them fail together and there is exactly
    one thing to fix. Per-session counting would also mean UNHEALTHY_AFTER had to
    be spent on every session before /health noticed anything -- fifteen failures
    instead of three -- and noticing quickly is the whole point of it.
    """

    __slots__ = ("index", "failures", "next_probe")

    def __init__(self, index: int) -> None:
        self.index = index
        self.failures = 0
        # Which of this account's sessions the keepalive should poke next.
        self.next_probe = 0


class Session:
    """One GeminiClient, with its own lock.

    A client holds one conversation-bearing HTTP session, so two requests sharing
    one interleave their state. A lock each -- rather than one lock over the pool
    -- is what lets N of them work at the same time.
    """

    __slots__ = ("account", "client", "lock")

    def __init__(self, account: Account, client: GeminiClient) -> None:
        self.account = account
        self.client = client
        self.lock = asyncio.Lock()


def _split(total: int, buckets: int) -> list[int]:
    """`total` sessions dealt as evenly as possible over `buckets` accounts.

    At least one each, even when there are more accounts than the limit allows:
    an account with no session is invisible to /health and to the round-robin,
    which would quietly turn a configured account into a missing one.
    """
    if buckets <= 0:
        return []
    base, extra = divmod(total, buckets)
    return [max(1, base + (1 if b < extra else 0)) for b in range(buckets)]


async def _open(psid: str, psidts: str) -> GeminiClient | None:
    """One initialised client, or None with the reason logged."""
    client = GeminiClient(psid, psidts)
    try:
        # auto_refresh=False is the load-bearing part -- see CONCURRENCY. The
        # keeper owns the rotating cookie; a client that refreshed it here would
        # invalidate the copy its siblings and the keeper's browser are using.
        await client.init(timeout=GEMINI_TIMEOUT, auto_refresh=False)
    except Exception as e:  # noqa: BLE001 - one bad session must not stop the rest
        log.error("a session failed to initialise: %s", str(e)[:160])
        return None
    return client


async def _prove(client: GeminiClient, index: int) -> bool:
    """Does this account actually work?

    `init()` is not a validity check: it succeeds for expired cookies and for
    outright garbage ("garbage:alsogarbage" was accepted, then failed every real
    call). Nor is a text prompt -- those still answer unauthenticated. Only an
    image prompt proves the account, which is exactly what /judge and /square
    need.

    Once per account, not once per session: the sessions on an account share its
    cookies, so a second probe re-tests the credentials the first one proved and
    spends another image generation from the same quota to do it. A session whose
    `init()` succeeded on already-proven cookies is as verified as it can be.
    """
    try:
        await client.generate_content(
            PROBE_PROMPT, files=[PROBE_PNG], model=JUDGE_MODEL, temporary=True
        )
    except Exception as e:  # noqa: BLE001 - one bad account must not stop the rest
        log.error("account %d is not usable: %s", index, str(e)[:160])
        return False
    return True


class Pool:
    """Round-robin over the initialised sessions, handing out an idle one.

    Tracks consecutive failures per account, because `init()` succeeding proves
    almost nothing: with expired cookies it still reports "Gemini client
    initialized successfully", logs `Account status: UNAUTHENTICATED` as a
    warning, and then fails every single call with error 1100. A healthcheck
    counting initialised clients therefore said "healthy" through a total
    outage -- observed in production, which is what prompted this.
    """

    def __init__(self) -> None:
        self.accounts: list[Account] = []
        self.sessions: list[Session] = []
        # Where the round-robin resumes. A plain counter rather than
        # itertools.cycle because the list is rebuilt whenever cookies change,
        # and a cycle would have to be rebuilt with it.
        self._next = 0

    async def start(self) -> None:
        self.accounts, self.sessions = await self._build(_accounts())
        self._next = 0
        # Said here rather than in _build, because only at boot does an empty
        # result mean the service has nothing: a failed *replace* logs the same
        # emptiness while the previous pool carries on screening.
        if not self.sessions:
            log.warning("no usable Gemini accounts; every request will return 502")

    async def replace(self, pairs: list[tuple[str, str]]) -> int:
        """Swap in new cookies and report how many accounts actually work.

        Builds and *proves* the new sessions before touching the live list, so a
        paste of dead cookies leaves the working ones in place rather than taking
        screening down in exchange for nothing.

        The new sessions are built into locals and swapped in one statement, which
        is the difference between "the old pool keeps working" and "screening is
        down for a few seconds". Clearing the list first and rebuilding into it
        looked equivalent and was not: proving one account is an `init()` plus a
        real image generation, and every upload arriving in that window got a 502,
        which the app records as an outage and publishes as held-for-review. The
        keeper pushes cookies every time __Secure-1PSIDTS rotates, so that window
        came round roughly every half hour rather than only when someone pasted
        into /admin.

        A call already in flight keeps the session it leased: it holds a reference,
        so dropping the list does not disturb it, and curl_cffi closes the handle
        when the last reference goes. Its `succeeded`/`failed` lands on the old
        Account object, which /health no longer reads -- deliberately, since that
        account's cookies have just been superseded.
        """
        accounts, sessions = await self._build(pairs)
        if not sessions:
            return 0
        self.accounts, self.sessions = accounts, sessions
        # Reset with the list it indexes into, not before building: _next is an
        # offset into `sessions` and the two have to change together.
        self._next = 0
        return len(accounts)

    async def _build(
        self, pairs: list[tuple[str, str]]
    ) -> tuple[list[Account], list[Session]]:
        """The new accounts and sessions, without publishing them. The caller
        decides when they become live -- see `replace`."""
        accounts: list[Account] = []
        sessions: list[Session] = []
        _forget_cached_cookies()
        for i, want in enumerate(_split(CONCURRENCY, len(pairs))):
            psid, psidts = pairs[i]
            first = await _open(psid, psidts)
            if first is None:
                # Said per account as well as per session: "a session failed" is
                # a degraded pool, "account 0 is not usable" is why screening is
                # down, and the log has to make that difference obvious.
                log.error("account %d is not usable: it would not initialise", i)
                continue
            if not await _prove(first, i):
                continue
            account = Account(i)
            # The rest in parallel. Each init() is several round-trips to Google,
            # and until the pool is up the app publishes uploads unscreened --
            # doing these one after another would multiply that window by
            # CONCURRENCY for nothing. Safe to overlap because with
            # auto_refresh=False none of them writes shared cookie state.
            extra = await asyncio.gather(
                *(_open(psid, psidts) for _ in range(want - 1))
            )
            clients = [first, *(c for c in extra if c is not None)]
            accounts.append(account)
            sessions.extend(Session(account, c) for c in clients)
            if len(clients) < want:
                log.warning(
                    "account %d ready with only %d of %d session(s)",
                    i,
                    len(clients),
                    want,
                )
            else:
                log.info("account %d ready with %d session(s)", i, len(clients))

        if not sessions:
            log.warning("none of the %d cookie pair(s) produced a usable session", len(pairs))
        return accounts, sessions

    @asynccontextmanager
    async def lease(self) -> AsyncIterator[Session | None]:
        """Hold one session for the duration of one call, then release it.

        A context manager rather than a pair of calls because forgetting the
        release would take a session out of circulation permanently, and there is
        no way to notice that except by watching throughput fall.

        Yields None when nothing is usable at all, which the callers turn into a
        502 rather than waiting for an account that may never arrive.
        """
        session = await self._take()
        try:
            yield session
        finally:
            if session is not None:
                session.lock.release()

    async def _take(self) -> Session | None:
        if not self.sessions:
            return None
        n = len(self.sessions)
        start, self._next = self._next % n, (self._next + 1) % n

        for k in range(n):
            session = self.sessions[(start + k) % n]
            if not session.lock.locked():
                # Not a race: nothing between the check and the acquire yields to
                # the event loop -- asyncio.Lock.acquire() returns without
                # awaiting when the lock is free and has no waiters -- so no other
                # task can slip in and take it.
                await session.lock.acquire()
                return session

        # Everything is busy, so wait rather than refusing: the caller is a
        # background job with a long timeout, and turning an upload away because
        # the sidecar is busy is worse than making it wait its turn. `start` moves
        # every call, so a burst of waiters spreads over the ring instead of
        # queueing behind one session.
        session = self.sessions[start]
        await session.lock.acquire()
        return session

    def idle_for(self, account: Account) -> Session | None:
        """An idle session on this account, rotating which one, or None.

        None when they are all busy, and the keepalive treats that as "no need":
        a session part-way through a real call is already proof the account
        answers.
        """
        mine = [s for s in self.sessions if s.account is account]
        if not mine:
            return None
        for k in range(len(mine)):
            session = mine[(account.next_probe + k) % len(mine)]
            if not session.lock.locked():
                account.next_probe = (account.next_probe + k + 1) % len(mine)
                return session
        return None

    def succeeded(self, account: Account) -> None:
        account.failures = 0

    def failed(self, account: Account) -> None:
        account.failures += 1
        if account.failures == UNHEALTHY_AFTER:
            log.error(
                "account %d has failed %d calls in a row; reporting unhealthy",
                account.index,
                UNHEALTHY_AFTER,
            )

    def usable(self) -> int:
        return sum(1 for a in self.accounts if a.failures < UNHEALTHY_AFTER)


def _authorised(key: str | None) -> bool:
    """Constant-time compare, and an unset SIDECAR_KEY refuses everything rather
    than allowing everything -- an empty shared secret is a misconfiguration, not
    a decision to run open."""
    return bool(SIDECAR_KEY) and bool(key) and hmac.compare_digest(key, SIDECAR_KEY)


pool = Pool()
app = FastAPI(title="gemini sidecar")


async def _keepalive() -> None:
    """Ask each account something trivial, forever, so the session stays live.

    One session per account per cycle, and a different one each time. Poking all
    of them would spend a generation per session out of a quota they all share,
    to learn what the first one already said; rotating still exercises each
    session over time. Failures are counted the same way a real call's are, so an
    expired session shows up in /health within minutes instead of at the next
    upload.
    """
    while True:
        await asyncio.sleep(KEEPALIVE_SECONDS)
        for account in list(pool.accounts):
            session = pool.idle_for(account)
            if session is None:
                continue
            try:
                async with session.lock:
                    # With the probe image, for the same reason _prove uses it: a
                    # text-only keepalive would keep reporting success against an
                    # unauthenticated session.
                    await session.client.generate_content(
                        PROBE_PROMPT,
                        files=[PROBE_PNG],
                        model=JUDGE_MODEL,
                        temporary=True,
                    )
                pool.succeeded(account)
                log.debug("keepalive ok for account %d", account.index)
            except Exception as e:  # noqa: BLE001
                pool.failed(account)
                log.warning("keepalive failed for account %d: %s", account.index, e)


@app.on_event("startup")
async def _startup() -> None:
    _write_probe()

    # Built in the background, not awaited.
    #
    # Uvicorn does not accept a single connection until this hook returns, and
    # building the pool means an image probe per account with GEMINI_TIMEOUT to
    # spare -- so a slow or hanging Google call made the whole sidecar refuse
    # connections rather than answer. Observed in production as `Connection
    # refused` on /health while the container reported "health: starting".
    #
    # Serving immediately is strictly better: /health honestly reports zero usable
    # accounts until the pool is ready (503, which the container healthcheck reads),
    # and an upload arriving in that window is *held* for review rather than
    # published unscreened. Both are states the system already handles.
    async def _warm() -> None:
        await pool.start()

    app.state.warmup = asyncio.create_task(_warm())
    # Held so they are not garbage collected mid-flight; never awaited, since they
    # run for the life of the process.
    app.state.keepalive = asyncio.create_task(_keepalive())


@app.get("/health")
async def health() -> Response:
    """`accounts` is what the container healthcheck reads, and it counts accounts
    that are actually answering -- not accounts that merely started up. 503 as
    well, so anything checking status rather than the body also sees it.

    `accounts` and `initialised` stay counts of *accounts*, not of sessions:
    /admin renders them as "N account(s) answering", and several sessions on one
    cookie pair are one account's worth of screening. `sessions` is reported
    separately, because "how much can run at once" is worth being able to see.
    """
    body = {
        "accounts": pool.usable(),
        "initialised": len(pool.accounts),
        "sessions": len(pool.sessions),
        "consecutive_failures": [a.failures for a in pool.accounts],
    }
    return JSONResponse(body, status_code=200 if pool.usable() else 503)


async def _spool(image: UploadFile) -> str:
    """Write the upload to a real path and hand back the path.

    A real path on disk, not BytesIO: gemini_webapi's uploader requires an
    explicit filename for in-memory data and `generate_content` gives no way to
    supply one, so a BytesIO silently uploads nothing and the model then answers
    in prose with no image attached. Observed exactly that before switching.

    In chunks rather than one `read()`, and *before* a session is leased. Both are
    about several uploads now being in flight together: a whole 12MB body per
    request in RAM adds up fast in a 512MB container, and a session held while a
    slow client finishes uploading is a session another upload could have used.
    """
    fd, src = tempfile.mkstemp(suffix=".png", dir="/tmp")
    with os.fdopen(fd, "wb") as f:
        while chunk := await image.read(1 << 20):
            f.write(chunk)
    return src


def _discard(path: str) -> None:
    try:
        os.unlink(path)
    except OSError:
        pass


@app.post("/cookies")
async def cookies(
    payload: dict,
    x_sidecar_key: str | None = Header(default=None),
) -> Response:
    """Swap in fresh cookies without a redeploy.

    This is how the keeper keeps screening alive: it watches a signed-in browser's
    cookie jar and posts the pair here whenever it rotates. The same endpoint
    backs the paste box in /admin, which is the manual override -- editing a
    GitHub secret and waiting out a ~12 minute CI deploy, versus pasting two
    values and having screening back in seconds.

    Verified before being accepted: the new cookies have to pass a real image
    probe, or the request is refused and whatever was working stays working.
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
    src = await _spool(image)
    try:
        async with pool.lease() as session:
            if session is None:
                return JSONResponse(
                    {"reason": "no Gemini account available"}, status_code=502
                )
            try:
                judged = await session.client.generate_content(
                    JUDGE_PROMPT, files=[src], model=JUDGE_MODEL, temporary=True
                )
                pool.succeeded(session.account)
            except Exception as e:  # noqa: BLE001
                pool.failed(session.account)
                log.error("judge call failed: %s", e)
                return JSONResponse(
                    {"reason": f"gemini judge failed: {e}"}, status_code=502
                )
    finally:
        _discard(src)

    lines = [l.strip().upper() for l in judged.text.splitlines() if l.strip()]
    verdict = lines[0] if lines else ""
    topic = lines[1] if len(lines) > 1 else ""
    log.info("verdict=%r topic=%r", verdict, topic)
    # Reported as-is, including an empty answer. The caller fails closed on
    # anything that is not an explicit PASS.
    return JSONResponse({"verdict": verdict, "topic": topic})


@app.post("/square")
async def square(image: UploadFile = File(...)) -> Response:
    """The slow one: Gemini editing the image into a 1:1 square."""
    src = await _spool(image)
    try:
        async with pool.lease() as session:
            if session is None:
                return JSONResponse(
                    {"reason": "no Gemini account available"}, status_code=502
                )
            resp = await _square(session.client, src)
            if resp.status_code == 200:
                pool.succeeded(session.account)
            else:
                pool.failed(session.account)
            return resp
    finally:
        _discard(src)


async def _square(client: GeminiClient, src: str) -> Response:
    try:
        edited = await client.generate_content(
            SQUARE_PROMPT, files=[src], model=IMAGE_MODEL, temporary=True
        )
    except Exception as e:  # noqa: BLE001
        log.error("image call failed: %s", e)
        return JSONResponse({"reason": f"gemini image failed: {e}"}, status_code=502)

    if not edited.images:
        # The model answered in prose instead of producing a picture. The caller
        # keeps the original rather than losing the upload.
        return JSONResponse({"reason": "gemini returned no image"}, status_code=502)

    # `save()` is the only way to get the bytes -- the Image type exposes no
    # reader -- so it round-trips through tmpfs and is deleted immediately. A uuid
    # in the name because several squares are in flight at once now, and anything
    # derived from the process or an object identity can repeat.
    path = None
    try:
        path = await edited.images[0].save(
            path="/tmp", filename=f"square-{uuid.uuid4().hex}.bin", verbose=False
        )
        with open(path, "rb") as f:
            out = f.read()
    finally:
        if path:
            _discard(path)

    # Whatever Gemini's container was (JPEG today), the app re-encodes to PNG on
    # the way to storage, so the format here is not load-bearing.
    return Response(content=out, media_type="application/octet-stream")
