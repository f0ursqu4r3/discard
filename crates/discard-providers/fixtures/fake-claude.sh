#!/usr/bin/env bash
# Test double for the `claude` CLI. Replays fixtures/claude-approval-turn.jsonl over stream-json.
# Env: FAKE_CLAUDE_MODE=approval|hang|exit_early   FAKE_CLAUDE_LOG=<file>   FAKE_CLAUDE_FIXTURE=<file>
#      FAKE_CLAUDE_CHILD_PID=<file>   FAKE_CLAUDE_LOGGED_IN=true|false
log() { [ -n "${FAKE_CLAUDE_LOG:-}" ] && printf '%s\n' "$*" >>"$FAKE_CLAUDE_LOG"; return 0; }

case "${1:-}" in
  --version) echo "9.9.9 (Fake Claude)"; exit 0 ;;
  auth) echo "{\"loggedIn\":${FAKE_CLAUDE_LOGGED_IN:-true},\"email\":\"never-read@example.invalid\"}"; exit 0 ;;
esac

log "ARGS $*"
mode="${FAKE_CLAUDE_MODE:-approval}"
fixture="${FAKE_CLAUDE_FIXTURE:?}"

if [ "$mode" = exit_early ]; then
  echo "No conversation found with session ID: nope" >&2
  exit 1
fi

# line 1 of the fixture is the init event
head -n 1 "$fixture"
IFS= read -r user_line; log "STDIN $user_line"

if [ "$mode" = hang ]; then
  sleep 300 &
  [ -n "${FAKE_CLAUDE_CHILD_PID:-}" ] && echo $! >"$FAKE_CLAUDE_CHILD_PID"
  wait
  exit 0
fi

req_line=$(grep -n '"type":"control_request"' "$fixture" | head -n 1 | cut -d: -f1)
sed -n "2,${req_line}p" "$fixture"
IFS= read -r response; log "STDIN $response"
sed -n "$((req_line + 1)),\$p" "$fixture"
