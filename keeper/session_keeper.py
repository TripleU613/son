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

**Logging in is a human step, once.** Nothing here types a password or handles
credentials: it drives a browser profile that is already authenticated. See
`keeper/README.md` for how to produce that profile, and `LOGIN_CHECK_ONLY=1` to
verify one before deploying it.

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

# Login mode: run the browser with a display instead of headless, so a human can
# sign in through it over the tailnet. See `login_mode()`.
LOGIN_MODE = os.environ.get("KEEPER_LOGIN_MODE") == "1"
LOGIN_MINUTES = int(os.environ.get("KEEPER_LOGIN_MINUTES", "20"))

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


async def run() -> None:
    # No browser until there is a profile to drive.
    #
    # Chromium idles at ~650MB, and this container's whole job is to use *a
    # logged-in session*. Launching it to sit in a failing loop with no profile
    # spent two thirds of the memory limit achieving nothing -- measured in
    # production before this changed. Polling R2 costs one HTTP request a minute
    # and means uploading the profile is enough to start it: no restart, no deploy.
    while not restore_profile():
        log.error(
            "no logged-in profile at r2://%s. Nothing here can log in -- sign in "
            "once (keeper/README.md) and upload it; this will pick it up within a "
            "minute. Not launching a browser until then.",
            PROFILE_KEY,
        )
        await asyncio.sleep(60)

    log.info("profile present; starting the browser")
    async with async_playwright() as pw:
        # A persistent context, not a fresh browser: the whole point is to keep
        # using one profile's session rather than starting a new anonymous one
        # that would need logging in.
        ctx = await pw.chromium.launch_persistent_context(
            PROFILE_DIR,
            headless=True,
            args=["--no-sandbox", "--disable-dev-shm-usage"],
        )
        page = ctx.pages[0] if ctx.pages else await ctx.new_page()
        last: tuple[str, str] | None = None

        while True:
            try:
                last = await _cycle(ctx, page, last)
            except Exception as e:  # noqa: BLE001 - a bad cycle must not kill the loop
                log.error("refresh cycle failed: %s", e)
            await asyncio.sleep(REFRESH_SECONDS)


async def login_mode() -> None:
    """Hold a visible browser open so a human can sign in, then save the profile.

    This is the one step nothing here can do on its own: Google mints the session,
    and only in response to a real login. Rather than asking for a password -- which
    this codebase will not handle -- or for a DevTools cookie hunt, the keeper's own
    Chromium runs with a display and is reachable over the tailnet. Sign in, and the
    profile it was already going to use is saved to R2.

    Starts from any existing profile, so this doubles as "re-authenticate the one I
    have" rather than only ever starting from scratch.

    Bounded by KEEPER_LOGIN_MINUTES: an interactive browser holding a live session
    should not stay open indefinitely because someone forgot to switch the mode off.
    It saves whatever it has when the window closes or the time runs out.
    """
    restore_profile()
    log.info(
        "login mode: open the VNC page, sign in to Google, then close the window "
        "or wait. Saving to r2://%s. Window: %d minutes.",
        PROFILE_KEY,
        LOGIN_MINUTES,
    )

    async with async_playwright() as pw:
        ctx = await pw.chromium.launch_persistent_context(
            PROFILE_DIR,
            headless=False,
            args=["--no-sandbox", "--disable-dev-shm-usage", "--start-maximized"],
        )
        page = ctx.pages[0] if ctx.pages else await ctx.new_page()
        await page.goto(GEMINI_URL, wait_until="domcontentloaded", timeout=90_000)

        deadline = asyncio.get_running_loop().time() + LOGIN_MINUTES * 60
        signed_in = False
        while asyncio.get_running_loop().time() < deadline:
            await asyncio.sleep(10)
            try:
                jar = {c["name"] for c in await ctx.cookies()}
            except Exception:  # noqa: BLE001 - the window was closed
                break
            if all(n in jar for n in WANTED):
                if not signed_in:
                    log.info("signed in; saving the profile")
                    signed_in = True
                # Saved on every pass while signed in, so closing the window at any
                # point leaves a good copy in R2 rather than only saving at the end.
                save_profile()
                jar_full = {c["name"]: c["value"] for c in await ctx.cookies()}
                await push_cookies(jar_full["__Secure-1PSID"], jar_full["__Secure-1PSIDTS"])

        if not signed_in:
            log.error("login window closed without a signed-in session; nothing saved")
        try:
            await ctx.close()
        except Exception:  # noqa: BLE001
            pass


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
    elif LOGIN_MODE:
        asyncio.run(login_mode())
    else:
        try:
            asyncio.run(run())
        finally:
            shutil.rmtree(PROFILE_DIR, ignore_errors=True)
