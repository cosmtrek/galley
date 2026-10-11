#!/usr/bin/env bash
# Build and (re)start a local Galley behind a cloudflared quick tunnel.
#
#   scripts/dev.sh [restart]   build, then restart Galley (the tunnel is kept)
#   scripts/dev.sh start       start whatever is not running, without rebuilding
#   scripts/dev.sh check       run the web and server checks from AGENTS.md
#   scripts/dev.sh status      show processes, public URL and health
#   scripts/dev.sh stop [--all]  stop Galley; --all also stops the tunnel
#   scripts/dev.sh tunnel      replace the tunnel (new URL) and restart Galley
#   scripts/dev.sh logs        follow the server log
#
# Env: GALLEY_ADDR (default 127.0.0.1:7860), GALLEY_DATA (default ./data),
# GALLEY_DEV_TUNNEL=0 to run without cloudflared.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DATA=$(mkdir -p "${GALLEY_DATA:-$ROOT/data}" && cd "${GALLEY_DATA:-$ROOT/data}" && pwd)
ADDR=${GALLEY_ADDR:-127.0.0.1:7860}
PORT=${ADDR##*:}
TUNNEL=${GALLEY_DEV_TUNNEL:-1}
BIN=$ROOT/server/target/release/galley
LOG=$DATA/server.log
RUN=$DATA/dev
GALLEY_PIDFILE=$RUN/galley.pid
TUNNEL_PIDFILE=$RUN/cloudflared.pid
TUNNEL_URLFILE=$RUN/tunnel.url
TUNNEL_LOG=$RUN/cloudflared.log
mkdir -p "$RUN"

say() { printf '==> %s\n' "$*"; }
die() { printf 'dev.sh: %s\n' "$*" >&2; exit 1; }
alive() { [[ -n ${1:-} ]] && kill -0 "$1" 2>/dev/null; }

# Run a command in its own session so it outlives the agent shell that called us. setsid -f returns
# at once; the lock fd and our stdout must not leak into the child, or the next run blocks on them.
detach() {
  local log=$1; shift
  setsid -f "$@" >>"$log" 2>&1 </dev/null 9>&-
}

# Several agent sessions share this checkout; serialize everything that builds or restarts.
lock() {
  exec 9>"$RUN/dev.lock"
  flock -w 900 9 || die "another dev.sh is still running"
}

# Whatever galley listens on our port, including ones started by hand or by older sessions.
galley_pid() {
  local pid
  pid=$(ss -ltnpH "sport = :$PORT" 2>/dev/null | sed -n 's/.*pid=\([0-9]*\).*/\1/p' || true)
  pid=${pid%%$'\n'*}
  [[ -n $pid && $(cat "/proc/$pid/comm" 2>/dev/null) == galley ]] && echo "$pid"
  return 0
}

tunnel_pid() {
  local pid
  pid=$(cat "$TUNNEL_PIDFILE" 2>/dev/null || true)
  if alive "$pid" && [[ $(tr '\0' ' ' <"/proc/$pid/cmdline") == *"cloudflared"*"--url http://$ADDR"* ]]; then
    echo "$pid"
  else
    pgrep -o -f "cloudflared tunnel .*--url http://$ADDR\$" || true
  fi
}

public_url() {
  if [[ -n ${GALLEY_PUBLIC_URL:-} ]]; then echo "$GALLEY_PUBLIC_URL"
  elif [[ $TUNNEL == 1 ]]; then cat "$TUNNEL_URLFILE"
  else echo "http://$ADDR"; fi
}

build() {
  say "building web"
  (cd "$ROOT/web" && corepack pnpm install --frozen-lockfile && corepack pnpm build)
  say "building server"
  (cd "$ROOT/server" && cargo build --release --locked)
}

check() {
  (cd "$ROOT/web" && corepack pnpm check && corepack pnpm test)
  (cd "$ROOT/server" && cargo test --locked)
}

ensure_tunnel() {
  [[ $TUNNEL == 1 ]] || return 0
  command -v cloudflared >/dev/null || die "cloudflared not found (or set GALLEY_DEV_TUNNEL=0)"
  local pid url
  pid=$(tunnel_pid)
  if alive "$pid"; then
    [[ -s $TUNNEL_URLFILE ]] || die "cloudflared (pid $pid) is running but its URL is unknown; run 'scripts/dev.sh tunnel' to replace it"
    echo "$pid" >"$TUNNEL_PIDFILE"
    say "tunnel kept: $(cat "$TUNNEL_URLFILE") (pid $pid)"
    return 0
  fi
  say "starting cloudflared"
  rm -f "$TUNNEL_URLFILE"
  : >"$TUNNEL_LOG"
  detach "$TUNNEL_LOG" cloudflared tunnel --no-autoupdate --url "http://$ADDR"
  for _ in $(seq 60); do
    url=$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' "$TUNNEL_LOG" | head -1 || true)
    [[ -n $url ]] && break
    sleep 0.5
  done
  pid=$(tunnel_pid)
  alive "$pid" || die "cloudflared exited, see $TUNNEL_LOG"
  echo "$pid" >"$TUNNEL_PIDFILE"
  [[ -n $url ]] || die "no tunnel URL after 30s, see $TUNNEL_LOG"
  echo "$url" >"$TUNNEL_URLFILE"
  say "tunnel started: $url (the URL changed; links built from the old one stop working)"
}

stop_pid() {
  local pid=$1
  alive "$pid" || return 0
  kill "$pid"
  for _ in $(seq 50); do alive "$pid" || return 0; sleep 0.2; done
  kill -9 "$pid" 2>/dev/null || true
}

stop_galley() {
  local pid
  pid=$(galley_pid)
  [[ -n $pid ]] || pid=$(cat "$GALLEY_PIDFILE" 2>/dev/null || true)
  if alive "$pid"; then say "stopping galley (pid $pid)"; stop_pid "$pid"; fi
  rm -f "$GALLEY_PIDFILE"
  [[ -z $(galley_pid) ]] || die "port $PORT is still taken by galley pid $(galley_pid)"
}

stop_tunnel() {
  local pid
  pid=$(tunnel_pid)
  if alive "$pid"; then say "stopping cloudflared (pid $pid)"; stop_pid "$pid"; fi
  rm -f "$TUNNEL_PIDFILE" "$TUNNEL_URLFILE"
}

start_galley() {
  [[ -x $BIN ]] || die "$BIN is missing, run 'scripts/dev.sh restart'"
  [[ -z $(ss -ltnH "sport = :$PORT") ]] || die "port $PORT is in use by something other than galley"
  local url
  url=$(public_url)
  say "starting galley on $ADDR (public URL $url)"
  # Credentials always come from <data>/secrets.json, never from the caller's environment.
  (cd "$ROOT" && detach "$LOG" env -u GALLEY_PASSWORD -u GALLEY_AGENT_TOKEN \
    GALLEY_ADDR="$ADDR" GALLEY_DATA="$DATA" GALLEY_PUBLIC_URL="$url" "$BIN")
  for _ in $(seq 50); do
    if curl -fs -o /dev/null "http://$ADDR/app"; then
      galley_pid >"$GALLEY_PIDFILE"
      say "galley is up (pid $(cat "$GALLEY_PIDFILE"))"
      return 0
    fi
    sleep 0.2
  done
  tail -20 "$LOG" >&2
  die "galley did not answer on http://$ADDR/app within 10s"
}

status() {
  local pid url code
  pid=$(galley_pid)
  if [[ -n $pid ]]; then
    url=$(tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null | sed -n 's/^GALLEY_PUBLIC_URL=//p')
    echo "galley:  pid $pid, public URL ${url:-http://$ADDR}"
    [[ $TUNNEL == 1 && -s $TUNNEL_URLFILE && $url != "$(cat "$TUNNEL_URLFILE")" ]] &&
      echo "  warning: public URL does not match the tunnel; run 'scripts/dev.sh restart'"
  else
    echo "galley:  not running"
  fi
  if [[ $TUNNEL == 1 ]]; then
    pid=$(tunnel_pid)
    if alive "$pid"; then echo "tunnel:  pid $pid, $(cat "$TUNNEL_URLFILE" 2>/dev/null || echo 'URL unknown')"
    else echo "tunnel:  not running"; fi
  fi
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://$ADDR/app" || true)
  echo "local:   http://$ADDR/app -> $code"
  if [[ $TUNNEL == 1 && -s $TUNNEL_URLFILE ]]; then
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 "$(cat "$TUNNEL_URLFILE")/app" || true)
    echo "public:  $(cat "$TUNNEL_URLFILE")/app -> $code"
  fi
}

case ${1:-restart} in
  restart) lock; build; ensure_tunnel; stop_galley; start_galley; status ;;
  start)
    lock
    [[ -x $BIN ]] || build
    ensure_tunnel
    if [[ -n $(galley_pid) ]]; then say "galley already running (pid $(galley_pid))"; else start_galley; fi
    status ;;
  check) check ;;
  status) status ;;
  stop) lock; stop_galley; [[ ${2:-} == --all ]] && stop_tunnel; true ;;
  tunnel) lock; stop_tunnel; ensure_tunnel; stop_galley; start_galley; status ;;
  logs) exec tail -n 50 -f "$LOG" ;;
  *) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
