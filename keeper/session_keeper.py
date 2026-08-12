"""Keeps a logged-in Gemini browser session alive and feeds its cookies to the
sidecar.

The problem this solves: the Gemini web client authenticates as a browser
session, and a session that nothing uses expires. Pasting cookies by hand works
but has to be redone every time, and the failure is silent -- uploads just start
getting held.

A real browser does not have that problem. A logged-in Chromium that loads
gemini.google.com every so often refreshes its own `__Secure-1PSIDTS` the way any
open tab does, and its cookie jar is always current. This reads that jar and POSTs
the values to the sidecar's /cookies endpoint.

**The browser is not launched from here.** `entrypoint.sh` starts an ordinary
Chromium and this attaches to it over CDP. That is not incidental: a
Playwright-launched browser carries --enable-automation and navigator.webdriver,
and Google refuses to sign in to one ("Couldn't sign you in / This browser or app
may not be secure"). Nothing here spoofs those signals -- the browser genuinely is
not automated, and CDP is used only to read cookies and navigate, which the login
check does not inspect.

**Logging in is a human step.** Nothing here types a password or handles
credentials -- it drives a browser, and a person signs in through it. The browser
runs on a virtual display exported over VNC, which the app proxies to signed-in
admins at `/admin/browser`, so signing in needs nothing but the deployed site.

Where the profile lives: R2, not the server. "No storage on the server" is a hard
rule for this project, and a browser profile is state. It is pulled into /tmp at
boot, and pushed back whenever the cookies change -- so the durable copy is in
Cloudflare like everything else, and a container restart resumes the same session
instead of needing a fresh login.
"""

from __future__ import annotations

import asyncio
import logging
import os
import shutil
import tarfile
import tempfile

import json

import boto3
import httpx
from botocore.config import Config
from playwright.async_api import async_playwright

log = logging.getLogger("session-keeper")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

GEMINI_URL = "https://gemini.google.com/app"
SIDECAR = os.environ.get("SIDECAR_URL", "http://gemini:8099")
SIDECAR_KEY = os.environ.get("SIDECAR_KEY", "")

# Well inside the ~30 minute __Secure-1PSIDTS rotation window, so the jar is
# never more than one cycle stale.
REFRESH_SECONDS = int(os.environ.get("KEEPER_REFRESH_SECONDS", "600"))

# While not signed in, look far more often. This is a cookie read over CDP with no
# navigation and no network, so it is nearly free -- and the sooner a fresh login is
# noticed, the smaller the window in which a container restart could lose it.
IDLE_REFRESH_SECONDS = int(os.environ.get("KEEPER_IDLE_REFRESH_SECONDS", "5"))

PROFILE_DIR = "/tmp/profile"
PROFILE_KEY = os.environ.get("KEEPER_PROFILE_KEY", "keeper/profile.tar.gz")

# The session itself, stored separately from the profile and authoritative over it.
#
# The profile turned out not to contain the session at all: Chromium keeps cookies in
# memory and writes its SQLite store lazily, so an archive taken from a running
# browser had eight Google cookies in it and neither of the two that matter. Verified
# by reading Default/Cookies out of the stored archive. Cookies read over CDP are
# exact and current, so those are what get saved, and they are injected back into the
# browser on boot rather than hoping it reads them off disk.
COOKIES_KEY = os.environ.get("KEEPER_COOKIES_KEY", "keeper/cookies.json")

# The two cookies gemini_webapi needs. The rest of the jar is irrelevant to it.
WANTED = ("__Secure-1PSID", "__Secure-1PSIDTS")

# Where entrypoint.sh put the browser's debugging endpoint. Loopback only.
CDP_URL = os.environ.get("KEEPER_CDP_URL", "http://127.0.0.1:9222")

# Failed re-attaches before the process gives up and lets Docker restart it. Three
# rather than one, because a browser mid-navigation can refuse a connection briefly
# without being gone.
MAX_BROKEN = 3


def _r2():
    """An S3 client pointed at R2, or None when R2 is not configured.

    boto3 rather than the aws CLI: `awscli` has no installation candidate on the
    Playwright image's Ubuntu base, and a pip dependency is one pinned line where
    apt was a whole layer plus a repository to enable.
    """
    needed = ("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY", "R2_BUCKET", "CF_ACCOUNT_ID")
    if not all(os.environ.get(k) for k in needed):
        return None
    return boto3.client(
        "s3",
        endpoint_url=f"https://{os.environ['CF_ACCOUNT_ID']}.r2.cloudflarestorage.com",
        aws_access_key_id=os.environ["R2_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["R2_SECRET_ACCESS_KEY"],
        # R2 wants SigV4 and has no regions; "auto" is what it expects.
        region_name="auto",
        config=Config(signature_version="s3v4", retries={"max_attempts": 3}),
    )


def _is_runtime_junk(name: str) -> bool:
    """Chromium's per-launch scratch, which must not be carried between containers.

    The Singleton* entries are symlinks to an absolute path under /tmp naming the
    machine and process that made them; they are recreated on every launch, and
    restoring one either fails the extract or points the new browser at a socket
    that does not exist.
    """
    base = name.rsplit("/", 1)[-1]
    return base.startswith("Singleton")


def _restorable(tar: tarfile.TarFile):
    """Members worth restoring: no symlinks, no hardlinks, no runtime scratch."""
    for member in tar.getmembers():
        if member.issym() or member.islnk() or _is_runtime_junk(member.name):
            continue
        yield member


def _archivable(info: tarfile.TarInfo) -> tarfile.TarInfo | None:
    """The same exclusions on the way out, so a fresh archive is clean to begin
    with rather than relying on the reader to cope."""
    if info.issym() or info.islnk() or _is_runtime_junk(info.name):
        return None
    return info


def restore_profile() -> bool:
    """Pull the profile out of R2. False means "no profile yet, log in first"."""
    s3 = _r2()
    if s3 is None:
        log.warning("R2 not configured; the profile will not survive a restart")
        return os.path.isdir(PROFILE_DIR)

    with tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False) as tmp:
        dest = tmp.name
    try:
        s3.download_file(os.environ["R2_BUCKET"], PROFILE_KEY, dest)
    except Exception as e:  # noqa: BLE001 - a missing profile is expected, not fatal
        log.warning("no stored profile (%s)", str(e)[:200])
        os.unlink(dest)
        return False

    os.makedirs(PROFILE_DIR, exist_ok=True)
    try:
        with tarfile.open(dest, "r:gz") as tar:
            # filter="data" refuses absolute paths and traversal. Keeping it, and
            # skipping the members it would refuse, rather than dropping it:
            # Chromium's profile contains SingletonSocket/SingletonLock/
            # SingletonCookie, which are symlinks to absolute paths outside the
            # profile. The filter raised OutsideDestinationError on the first of
            # them and aborted the whole restore -- so a container restart lost a
            # session that was sitting safely in R2, which is the one thing this
            # function exists to prevent.
            tar.extractall(PROFILE_DIR, members=_restorable(tar), filter="data")
    except Exception as e:  # noqa: BLE001
        # A corrupt or unreadable archive is "no profile", not a crash. Raising here
        # killed the container before the browser was ever started, so nobody could
        # even sign in again to recover.
        log.error("stored profile could not be unpacked (%s); treating as absent", e)
        os.unlink(dest)
        return False
    os.unlink(dest)
    log.info("profile restored from R2")
    return True


def save_profile() -> None:
    """Push the profile back, so the next container resumes this session."""
    s3 = _r2()
    if s3 is None:
        return
    with tempfile.NamedTemporaryFile(suffix=".tar.gz", delete=False) as tmp:
        dest = tmp.name
    try:
        with tarfile.open(dest, "w:gz") as tar:
            tar.add(PROFILE_DIR, arcname=".", filter=_archivable)
        s3.upload_file(dest, os.environ["R2_BUCKET"], PROFILE_KEY)
        log.info("profile saved to R2")
    except Exception as e:  # noqa: BLE001
        log.error("could not save profile: %s", str(e)[:200])
    finally:
        os.unlink(dest)


def save_jar(cookies: list[dict]) -> None:
    """Store the whole cookie jar, not just the two the sidecar needs.

    Google's session is not only PSID/PSIDTS -- SID, HSID, SSID, SAPISID and friends
    are part of what makes a restored session actually work, and a jar missing them
    tends to bounce straight to a sign-in page. Storing everything costs a few KB.
    """
    s3 = _r2()
    if s3 is None:
        return
    try:
        s3.put_object(
            Bucket=os.environ["R2_BUCKET"],
            Key=COOKIES_KEY,
            Body=json.dumps(cookies).encode(),
            ContentType="application/json",
        )
        log.info("session jar saved to R2 (%d cookies)", len(cookies))
    except Exception as e:  # noqa: BLE001
        log.error("could not save the session jar: %s", str(e)[:200])


def load_jar() -> list[dict] | None:
    s3 = _r2()
    if s3 is None:
        return None
    try:
        body = s3.get_object(Bucket=os.environ["R2_BUCKET"], Key=COOKIES_KEY)["Body"].read()
        jar = json.loads(body)
        log.info("session jar loaded from R2 (%d cookies)", len(jar))
        return jar
    except Exception as e:  # noqa: BLE001 - absent is the normal first-run case
        log.info("no stored session jar (%s)", str(e)[:120])
        return None


async def push_cookies(psid: str, psidts: str) -> bool:
    async with httpx.AsyncClient(timeout=120) as client:
        try:
            r = await client.post(
                f"{SIDECAR}/cookies",
                json={"cookies": f"{psid}:{psidts}"},
                headers={"X-Sidecar-Key": SIDECAR_KEY},
            )
        except Exception as e:  # noqa: BLE001
            log.error("could not reach the sidecar: %s", e)
            return False
    if r.status_code == 200:
        log.info("sidecar accepted the cookies: %s", r.text.strip())
        return True
    log.error("sidecar refused the cookies (%s): %s", r.status_code, r.text[:200])
    return False


async def _cycle(
    ctx, page, last: tuple[str, str] | None
) -> tuple[str, str] | None:
    """Read the cookie jar; navigate first only if there is a session to refresh.

    The navigation is conditional and that is the whole point. While nobody is
    signed in yet, this used to reload gemini.google.com on every pass -- which
    means reloading the page out from under whoever is part-way through typing a
    password into it, wiping the form and sending them back to the start. Reading
    cookies over CDP needs no page load at all, so while waiting it only looks.

    Once signed in the navigation is what makes Google reissue the rotating cookie,
    so it happens then, on the slow cadence.
    """
    if last is not None:
        await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
        await page.wait_for_timeout(4_000)

    jar = {c["name"]: c["value"] for c in await ctx.cookies()}
    missing = [n for n in WANTED if n not in jar]
    if missing:
        # Signed out, or not signed in yet. Reported by the caller, which knows
        # whether this is news.
        return None

    pair = (jar["__Secure-1PSID"], jar["__Secure-1PSIDTS"])
    if pair == last:
        log.debug("cookies unchanged")
        return last

    # The jar first: it is what actually restores a session, where the profile
    # demonstrably does not.
    save_jar(await ctx.cookies())

    # Saved *before* the sidecar is told, and regardless of what it says.
    #
    # It used to be the other way round -- push first, save only if the push was
    # accepted -- which threw away a perfectly good login whenever the sidecar was
    # unhappy, and lost it entirely on the next restart because the profile lives in
    # a tmpfs. A signed-in profile is the valuable artifact here and the expensive
    # one to replace: it needs a human. The sidecar can always be told again.
    log.info("cookies changed; saving the profile")
    save_profile()
    await push_cookies(*pair)
    return pair


async def _park(page) -> None:
    """Unload the Gemini page between cycles.

    The SPA is most of the browser's footprint, and it only needs to be resident
    for the seconds it takes to refresh the cookie. Parking on about:blank gives
    that memory back and costs one extra navigation per cycle.

    Only done once signed in: while a human still has to sign in, the page needs to
    be on screen for them to do it through.
    """
    try:
        await page.goto("about:blank", wait_until="domcontentloaded", timeout=15_000)
    except Exception as e:  # noqa: BLE001 - parking is an optimisation, never fatal
        log.debug("could not park the page: %s", e)


async def _attach(pw):
    """Attach to the browser entrypoint.sh started, and hand back a usable page.

    Separate from `run` so a dead browser can be recovered mid-loop: Chromium
    crashing or being closed leaves every Playwright handle permanently broken, and
    without re-attaching the keeper would sit there failing every cycle until
    somebody noticed.
    """
    browser = await pw.chromium.connect_over_cdp(CDP_URL)
    ctx = browser.contexts[0] if browser.contexts else await browser.new_context()
    page = ctx.pages[0] if ctx.pages else await ctx.new_page()
    log.info("attached to the browser over CDP at %s", CDP_URL)
    return ctx, page


async def run() -> None:
    """Attach to the browser and keep its session fresh.

    Not headless, and started whether or not a profile exists yet. Both are
    deliberate: this is the window an admin signs in through, and it has to be
    there *before* there is a session, because creating the session is what they
    are opening it for. A mode flag would put a redeploy between "the session
    died" and "I can fix it".

    The loop is the same either way -- load the page, read the jar, push the
    cookies if they moved -- and simply reports that it is not signed in until it
    is. The moment someone signs in through the VNC window, the next cycle sees
    new cookies and hands them to the sidecar.
    """
    # No restore here: entrypoint.sh does it before launching the browser, which is
    # the only order that works. Restoring now would write into a profile directory
    # Chromium already has open, and it would never read those files.
    async with async_playwright() as pw:
        ctx, page = await _attach(pw)

        # A restored profile does not carry the session (see COOKIES_KEY), so the
        # stored jar is injected straight into the live browser. Only when it is not
        # already signed in: overwriting a good live session with a stale stored one
        # would be a downgrade.
        try:
            live = {c["name"] for c in await ctx.cookies()}
            if not all(n in live for n in WANTED):
                jar = load_jar()
                if jar:
                    await ctx.add_cookies(jar)
                    log.info("injected the stored session; reloading to confirm")
                    await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
                    await page.wait_for_timeout(4_000)
        except Exception as e:  # noqa: BLE001 - a failed injection must not stop the loop
            log.error("could not inject the stored session: %s", e)

        last: tuple[str, str] | None = None
        announced = False
        # Consecutive cycles that could not even re-attach. Past the limit the
        # process exits so Docker restarts it, which runs the entrypoint again and
        # gets a fresh browser -- the one thing this cannot fix from inside.
        broken = 0

        while True:
            try:
                before = last
                last = await _cycle(ctx, page, last)
                if last and last != before:
                    announced = False
                if last is None and not announced:
                    # Once, not every pass: at a five-second cadence this would be
                    # 700 identical lines an hour, which buries everything else.
                    log.error(
                        "not signed in. Open /admin/browser on the site and sign in "
                        "to Google in that window; it will be picked up within %d "
                        "seconds and saved immediately.",
                        IDLE_REFRESH_SECONDS,
                    )
                    announced = True
                broken = 0
            except Exception as e:  # noqa: BLE001 - a bad cycle must not kill the loop
                log.error("refresh cycle failed: %s", e)
                # Most likely the browser went away, which leaves every handle
                # permanently broken. Try to pick up a new one before giving up on
                # the cycle.
                try:
                    ctx, page = await _attach(pw)
                    broken = 0
                except Exception as attach_error:  # noqa: BLE001
                    broken += 1
                    log.error(
                        "could not re-attach (%d/%d): %s", broken, MAX_BROKEN, attach_error
                    )
                    if broken >= MAX_BROKEN:
                        log.error(
                            "browser unreachable %d times; exiting so the container "
                            "restarts with a fresh one",
                            broken,
                        )
                        raise SystemExit(1)

            if last is not None:
                # Signed in: give the page's memory back and settle into the slow
                # cadence.
                await _park(page)
                await asyncio.sleep(REFRESH_SECONDS)
            elif broken:
                # Mid-recovery: back off rather than hammering a browser that is
                # not answering.
                await asyncio.sleep(IDLE_REFRESH_SECONDS * 4)
            else:
                # Still waiting on a human: leave Gemini on screen for them and
                # come back quickly so their sign-in is noticed straight away.
                await asyncio.sleep(IDLE_REFRESH_SECONDS)


if __name__ == "__main__":
    if os.environ.get("LOGIN_CHECK_ONLY"):
        # Verify a profile before trusting it in production: restores, loads the
        # page once, and reports whether it is signed in.
        async def check() -> None:
            restore_profile()
            async with async_playwright() as pw:
                ctx = await pw.chromium.launch_persistent_context(
                    PROFILE_DIR, headless=True, args=["--no-sandbox"]
                )
                page = ctx.pages[0] if ctx.pages else await ctx.new_page()
                await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
                await page.wait_for_timeout(4_000)
                jar = {c["name"] for c in await ctx.cookies()}
                have = [n for n in WANTED if n in jar]
                print(f"  signed in: {len(have) == len(WANTED)}  (found {have})")
                await ctx.close()

        asyncio.run(check())
    else:
        try:
            asyncio.run(run())
        finally:
            shutil.rmtree(PROFILE_DIR, ignore_errors=True)
