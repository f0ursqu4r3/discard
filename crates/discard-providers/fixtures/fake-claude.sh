#!/usr/bin/env bash
# Test double for the `claude` CLI. Replays fixtures/claude-approval-turn.jsonl over stream-json.
# Env: FAKE_CLAUDE_MODE=approval|hang|exit_early   FAKE_CLAUDE_LOG=<file>   FAKE_CLAUDE_FIXTURE=<file>
#      FAKE_CLAUDE_CHILD_PID=<file>   FAKE_CLAUDE_LOGGED_IN=true|false
log() { [ -n "${FAKE_CLAUDE_LOG:-}" ] && printf '%s\n' "$*" >>"$FAKE_CLAUDE_LOG"; return 0; }

if [ "${FAKE_CLAUDE_HANG_SETUP:-}" = "${1:-}" ]; then
  echo $$ >"$FAKE_CLAUDE_CHILD_PID"
  exec sleep 300
fi

case "${1:-}" in
  --version) echo "${FAKE_CLAUDE_VERSION:-2.1.318 (Fake Claude)}"; exit "${FAKE_CLAUDE_VERSION_EXIT:-0}" ;;
  --help) echo "${FAKE_CLAUDE_HELP:---tools --allowedTools --strict-mcp-config --restricted --disable-slash-commands --system-prompt-snapshot --settings --setting-sources --input-format --output-format --verbose --include-partial-messages --permission-mode --permission-prompts --permission-prompt-tool --effort --model --append-system-prompt --resume --mcp-config}"; exit 0 ;;
  auth) echo "{\"loggedIn\":${FAKE_CLAUDE_LOGGED_IN:-true},\"email\":\"never-read@example.invalid\"}"; exit 0 ;;
esac

log "ARGS $*"
log "CWD $PWD"
log "GIT_LOCKS ${GIT_OPTIONAL_LOCKS:-}"
log "MEMORY_ENV ${CLAUDE_CODE_DISABLE_AUTO_MEMORY:-} ${CLAUDE_CODE_DISABLE_CLAUDE_MDS:-} ${CLAUDE_CODE_PLUGIN_DIRS:-}"
log "AUTO_COMPACT ${DISABLE_AUTO_COMPACT:-}"
mode="${FAKE_CLAUDE_MODE:-approval}"
fixture="${FAKE_CLAUDE_FIXTURE:?}"

if [ "$mode" = exit_early ]; then
  echo "No conversation found with session ID: nope" >&2
  exit 1
fi

if [ "$mode" = resume_rejected ]; then
  echo "No conversation found with session ID: gone-1" >&2
  echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: gone-1"],"usage":{"input_tokens":0,"output_tokens":0}}'
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
