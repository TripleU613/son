#!/usr/bin/env bash
# A phone-shaped virtual display running an ordinary Chromium, exported over VNC.
#
# Two things here are deliberate and both come from watching a real sign-in fail.
#
# 1. Chromium is launched *by this script*, not by Playwright. Playwright starts a
#    browser with --enable-automation and navigator.webdriver = true, and Google
#    refuses to sign in from one: "Couldn't sign you in / This browser or app may
#    not be secure." Nothing here spoofs those signals -- the browser simply is not
#    automated. The keeper attaches over CDP afterwards only to read the cookie jar
#    and navigate, which is not something Google's login check looks at.
#
# 2. The display is phone-shaped and the UA is a phone's, because this is looked at
#    from a phone over VNC. A 1280x800 desktop scaled onto a handset is unusable for
#    typing a password, and Google's mobile sign-in layout is built for exactly the
#    screen the person is holding.
set -euo pipefail

export DISPLAY=:99

# Portrait, near enough to a phone's aspect that noVNC's scaling is close to 1:1.
Xvfb :99 -screen 0 "${KEEPER_SCREEN:-440x920x24}" -nolisten tcp &
for _ in $(seq 1 50); do
  [ -e /tmp/.X11-unix/X99 ] && break
  sleep 0.2
done

# A window manager, or Google's sign-in popups appear undecorated and unfocusable.
fluxbox >/dev/null 2>&1 &

# -localhost, so VNC is not on the network: websockify is the only reader, and it
# is reachable solely from the app container, which requires an admin session per
# request. -nopw for the same reason -- a password here would guard a port nothing
# can route to.
x11vnc -display :99 -forever -shared -localhost -nopw -quiet >/dev/null 2>&1 &

websockify --web /usr/share/novnc 6080 localhost:5900 >/dev/null 2>&1 &

# The browser Playwright shipped, started as a normal browser. Asking Playwright
# for the path rather than globbing /ms-playwright, so an image update that moves
# it does not silently break this.
CHROME="$(python -c 'from playwright.sync_api import sync_playwright
with sync_playwright() as p: print(p.chromium.executable_path)')"

mkdir -p /tmp/profile
# --remote-debugging-port on loopback only. Flags mirror the footprint work in
# session_keeper.py: no GPU exists in this container, and a renderer cap plus a
# small JS heap keep one page from growing without bound.
"$CHROME" \
  --remote-debugging-port=9222 \
  --remote-debugging-address=127.0.0.1 \
  --user-data-dir=/tmp/profile \
  --no-sandbox \
  --disable-dev-shm-usage \
  --disable-gpu \
  --disable-software-rasterizer \
  --renderer-process-limit=2 \
  --js-flags=--max-old-space-size=192 \
  --disable-extensions \
  --no-first-run \
  --no-default-browser-check \
  --disable-features=TranslateUI \
  --window-position=0,0 \
  --window-size="${KEEPER_WINDOW:-440,880}" \
  --user-agent="${KEEPER_UA:-Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36}" \
  "https://gemini.google.com/app" >/dev/null 2>&1 &

# Wait for CDP before the keeper tries to attach, rather than making it retry.
for _ in $(seq 1 100); do
  if python -c "import urllib.request,sys; urllib.request.urlopen('http://127.0.0.1:9222/json/version', timeout=1)" 2>/dev/null; then
    break
  fi
  sleep 0.3
done

exec python session_keeper.py
