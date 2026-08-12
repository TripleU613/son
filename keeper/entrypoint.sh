#!/usr/bin/env bash
# Bring up a virtual display, then run the keeper inside it.
#
# The browser is always visible, with no mode to switch: the whole point is that
# an admin can open /admin/browser on the deployed site at any moment and sign in
# to the very session the keeper refreshes. A flag to turn this on would mean a
# redeploy stands between "the session died" and "I can fix it", which is the
# friction this is meant to remove.
#
# The pieces: Xvfb for Chromium to draw into, fluxbox because Google's sign-in
# popups are unfocusable without a window manager, x11vnc to export the display,
# and websockify to make that a WebSocket the app can proxy. Together ~25MB.
set -euo pipefail

export DISPLAY=:99

# -nolisten tcp: the X server is for this container only, never the network.
Xvfb :99 -screen 0 1440x900x24 -nolisten tcp &
# Wait for the socket rather than sleeping a guessed number of seconds.
for _ in $(seq 1 50); do
  [ -e /tmp/.X11-unix/X99 ] && break
  sleep 0.2
done

fluxbox >/dev/null 2>&1 &

# -localhost, so VNC is not on the network at all: websockify below is the only
# reader, and it is only reachable from the app container, which requires an admin
# session for every request. -nopw for the same reason -- a password here would be
# a second secret guarding a port nothing can route to.
x11vnc -display :99 -forever -shared -localhost -nopw -quiet >/dev/null 2>&1 &

websockify --web /usr/share/novnc 6080 localhost:5900 >/dev/null 2>&1 &

exec python session_keeper.py
