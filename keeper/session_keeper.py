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

# While not signed in, cycle much faster. Two reasons: the Gemini page needs to be
# on screen for someone to sign in through, and once they do, the new session
# should be picked up in seconds rather than after a ten-minute wait.
IDLE_REFRESH_SECONDS = int(os.environ.get("KEEPER_IDLE_REFRESH_SECONDS", "45"))

# Chromium flags chosen for footprint. Measured in production at 1.07GB of a
# 1.17GB limit with the Gemini SPA resident -- 91%, which a sign-in page load would
# have pushed into an OOM kill mid-flow. There is no GPU in the container, so the
# GPU process and the software rasteriser behind it are pure overhead, and a
# renderer cap plus a smaller JS heap keeps one page from growing without bound.
CHROME_ARGS = [
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--start-maximized",
    "--disable-gpu",
    "--disable-software-rasterizer",
    "--renderer-process-limit=2",
    "--js-flags=--max-old-space-size=192",
    "--disable-extensions",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-features=TranslateUI",
]

PROFILE_DIR = "/tmp/profile"
PROFILE_KEY = os.environ.get("KEEPER_PROFILE_KEY", "keeper/profile.tar.gz")

# The two cookies gemini_webapi needs. The rest of the jar is irrelevant to it.
WANTED = ("__Secure-1PSID", "__Secure-1PSIDTS")


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
    with tarfile.open(dest, "r:gz") as tar:
        # filter="data" refuses absolute paths and traversal in member names. The
        # archive is ours, but an extract that trusts its input is a bad habit to
        # write down.
        tar.extractall(PROFILE_DIR, filter="data")
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
            tar.add(PROFILE_DIR, arcname=".")
        s3.upload_file(dest, os.environ["R2_BUCKET"], PROFILE_KEY)
        log.info("profile saved to R2")
    except Exception as e:  # noqa: BLE001
        log.error("could not save profile: %s", str(e)[:200])
    finally:
        os.unlink(dest)


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


async def _cycle(ctx, page, last: tuple[str, str] | None) -> tuple[str, str] | None:
    """One refresh: load the page, read the jar, push the cookies if they moved."""
    # A real navigation, which is what makes Google reissue the rotating cookie --
    # reading the jar without loading anything would never refresh it.
    await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)
    await page.wait_for_timeout(4_000)

    jar = {c["name"]: c["value"] for c in await ctx.cookies()}
    signed_in_now = all(n in jar for n in WANTED)
    missing = [n for n in WANTED if n not in jar]
    if missing:
        # Signed out: the profile is no longer authenticated and only a human can
        # fix that. Said plainly rather than retried silently forever.
        log.error(
            "profile is not signed in (missing %s). Re-do the one-time login and "
            "re-upload the profile.",
            ", ".join(missing),
        )
        return last

    pair = (jar["__Secure-1PSID"], jar["__Secure-1PSIDTS"])
    if pair == last:
        log.debug("cookies unchanged")
        return last

    log.info("cookies changed; handing them to the sidecar")
    if await push_cookies(*pair):
        save_profile()
        return pair
    return last


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


async def run() -> None:
    """One browser, always running, visible at /admin/browser.

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
    restore_profile()

    async with async_playwright() as pw:
        # A persistent context, so the session survives restarts via the profile,
        # and headless=False because there is a display and a human may be looking
        # at it.
        ctx = await pw.chromium.launch_persistent_context(
            PROFILE_DIR,
            headless=False,
            args=CHROME_ARGS,
            viewport=None,
        )
        page = ctx.pages[0] if ctx.pages else await ctx.new_page()
        last: tuple[str, str] | None = None
        announced = False

        while True:
            try:
                before = last
                last = await _cycle(ctx, page, last)
                if last and last != before:
                    announced = False
                if last is None and not announced:
                    log.error(
                        "not signed in. Open /admin/browser on the site and sign in "
                        "to Google in that window; this will pick it up within %d "
                        "seconds.",
                        IDLE_REFRESH_SECONDS,
                    )
                    announced = True
            except Exception as e:  # noqa: BLE001 - a bad cycle must not kill the loop
                log.error("refresh cycle failed: %s", e)

            if last is not None:
                # Signed in: give the page's memory back and settle into the slow
                # cadence.
                await _park(page)
                await asyncio.sleep(REFRESH_SECONDS)
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
