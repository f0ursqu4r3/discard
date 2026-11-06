#!/usr/bin/env python3
"""Test double for `codex`. Speaks enough of the app-server JSON-RPC protocol to drive CodexProvider.

Env: FAKE_CODEX_MODE=approval|hang|exit_early|resume_reject|unsupported   FAKE_CODEX_LOG=<file>
     FAKE_CODEX_FIXTURE=<file>   FAKE_CODEX_CHILD_PID=<file>   FAKE_CODEX_LOGGED_IN=true|false
"""
import json, os, subprocess, sys, time, re

args = sys.argv[1:]
mode = os.environ.get("FAKE_CODEX_MODE", "approval")
logfile = os.environ.get("FAKE_CODEX_LOG")


def log(text):
    if logfile:
        with open(logfile, "a") as f:
            f.write(text + "\n")


if args[:1] == ["--version"] and os.environ.get("FAKE_CODEX_HANG_SETUP"):
    with open(os.environ["FAKE_CODEX_CHILD_PID"], "w") as f:
        f.write(str(os.getpid()))
    time.sleep(300)

if args[:1] == ["--version"]:
    print(os.environ.get("FAKE_CODEX_VERSION", "codex-cli 0.166.0"))
    sys.exit(0)
if args[:2] == ["login", "status"]:
    if os.environ.get("FAKE_CODEX_LOGGED_IN", "true") == "true":
        print("Logged in using ChatGPT")
        sys.exit(0)
    print("Not logged in", file=sys.stderr)
    sys.exit(1)

log("ARGS " + " ".join(args))
log("CWD " + os.getcwd())
effective = json.loads(os.environ.get("FAKE_CODEX_CONFIG", '{}'))
for i, arg in enumerate(args[:-1]):
    if arg == "-c":
        key, raw_value = args[i + 1].split("=", 1)
        keys = [json.loads(part) if part.startswith('"') else part for part in re.findall(r'"(?:[^"\\]|\\.)*"|[^.]+', key)]
        target = effective
        for part in keys[:-1]: target = target.setdefault(part, {})
        value = json.loads(re.sub(r'("(?:[^"\\]|\\.)*")\s*=', r'\1:', raw_value))
        def merge(target, source):
            for key, value in source.items():
                if isinstance(value, dict): merge(target.setdefault(key, {}), value)
                else: target[key] = value
        merge(target, {keys[-1]: value})
if mode == "control_override": effective.setdefault("features", {})["shell_tool"] = True
if mode == "exit_early":
    sys.stderr.write("codex: failed to start: invalid config\n")
    sys.exit(1)


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def read():
    line = sys.stdin.readline()
    if not line:
        sys.exit(0)
    log("STDIN " + line.strip())
    return json.loads(line)


init = read()
send({"id": init["id"], "result": {"userAgent": "fake", "codexHome": "/tmp/fake-codex"}})
read()  # the `initialized` notification
thread = read()
if thread["method"] == "model/list":
    refresh = os.environ.get("FAKE_CODEX_REFRESH_CACHE")
    if refresh == "fail":
        send({"id": thread["id"], "error": {"code": -32603, "message": "Synthetic model refresh failure"}})
    else:
        if refresh:
            with open(refresh) as source:
                cache = json.load(source)
            with open(os.path.join(os.environ["CODEX_HOME"], "models_cache.json"), "w") as destination:
                json.dump(cache, destination)
        send({"id": thread["id"], "result": {"data": [{"id": "exact-model", "model": "exact-model", "displayName": "Exact model"}], "nextCursor": None}})
    while True:
        read()
while thread["method"] == "config/read":
    send({"id":thread["id"], "result":{"config":effective,"origins":{},"layers":[{"config":effective}]}})
    thread = read()
if thread["method"] == "thread/resume" and mode == "resume_reject":
    send({"id": thread["id"], "error": {"code": -32600, "message": "no rollout found for thread id " + thread["params"]["threadId"]}})
    sys.exit(0)
thread_id = thread["params"].get("threadId", "thread-fake-1")
send({"id": thread["id"], "result": {"thread": {"id": thread_id}}})
turn = read()
send({"id": turn["id"], "result": {"turn": {"id": "turn-1"}}})

if mode == "hang":
    send({"method": "turn/started", "params": {"threadId": thread_id, "turn": {"id": "turn-1"}}})
    child = subprocess.Popen(["sleep", "300"])
    pid_file = os.environ.get("FAKE_CODEX_CHILD_PID")
    if pid_file:
        with open(pid_file, "w") as f:
            f.write(str(child.pid))
    time.sleep(300)
    sys.exit(0)

if mode == "unsupported":
    send({"id": 77, "method": "item/permissions/requestApproval", "params": {}})
    read()

with open(os.environ["FAKE_CODEX_FIXTURE"]) as f:
    for line in f:
        msg = json.loads(line)
        send(msg)
        if msg.get("method") == "item/commandExecution/requestApproval":
            read()  # wait for the host's answer
