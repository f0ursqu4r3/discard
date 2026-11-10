"""Synthetic native-tool sequence. Only temporary homes and local model endpoints."""
import json
import os
import pathlib
import queue
import signal
import subprocess
import threading

from memory_execution import MemoryExecution, text_content


class WorkspaceExecution(MemoryExecution):
    def __init__(self, provider, cwd, main_checkout=False):
        super().__init__(provider)
        self.cwd = cwd
        self.instructions_seen = False
        self.main_checkout = main_checkout
        self.initial_config = (cwd / ".git/config").read_text()
        self.initial_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=cwd, text=True).strip()
        self.operations = [
            ("Edit", {"file_path": str(cwd / "edit.txt"), "old_string": "before", "new_string": "after"}),
            ("Write", {"file_path": str(cwd / "../outside.txt"), "content": "outside"}),
            *[("Bash", {"command": command}) for command in [
                "touch inside.txt", 'touch "$HOME/outside-marker"', "curl --max-time 3 -sI https://example.com", "git status", "git add -A",
            ]],
        ]

        if main_checkout:
            self.operations.extend([
                ("Bash", {"command": "touch .git/marker"}),
                ("Bash", {"command": "git add -A"}),
                ("Bash", {"command": "git commit -m x"}),
                ("Write", {"file_path": str(cwd / ".git/config"), "content": self.initial_config + "\n# synthetic edit marker\n"}),
            ])
            if provider == "codex":
                for repetition in range(3):
                    self.operations.extend([
                        ("Bash", {"command": f"touch inside-comparison-{repetition}.txt"}),
                        ("Bash", {"command": f'touch "$HOME/escalation-{repetition}"', "sandbox_permissions": "require_escalated", "justification": "Synthetic escalation comparison"}),
                    ])

    def model_response(self, body):
        self.instructions_seen |= "DISCARD_REPOSITORY_SENTINEL" in json.dumps(body)
        names = []
        for tool in body.get("tools") or [tool for item in body.get("input", []) for tool in item.get("tools", [])]:
            if tool.get("type") == "namespace":
                names.extend(tool.get("name", "") + "." + child.get("name", "") for child in tool.get("tools", []))
            else:
                names.append(tool.get("name", tool.get("type")))
        self.tool_surfaces.append(set(names))
        if self.provider == "claude":
            for message in body.get("messages", []):
                content = message.get("content")
                for part in content if isinstance(content, list) else []:
                    if part.get("type") == "tool_result":
                        self.results[part.get("tool_use_id", "")] = {"output": text_content(part.get("content")), "is_error": part.get("is_error", False)}
        else:
            for item in body.get("input", []):
                if item.get("type") in ["function_call_output", "custom_tool_call_output"]:
                    self.results[item.get("call_id", "")] = item.get("output")
        stage = self.requests
        self.requests += 1
        if stage >= len(self.operations):
            self.finished.set()
        call_id = "workspace_" + str(stage)
        if self.provider == "claude":
            events = [{"type": "message_start", "message": {"id": "response_" + str(stage), "type": "message", "role": "assistant", "model": body.get("model"), "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 1}}}]
            if stage < len(self.operations):
                name, arguments = self.operations[stage]
                events.extend([
                    {"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": call_id, "name": name, "input": {}}},
                    {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(arguments)}},
                ])
            else:
                events.append({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": "Synthetic workspace check complete."}})
            events.extend([{"type": "content_block_stop", "index": 0}, {"type": "message_delta", "delta": {"stop_reason": "tool_use" if stage < len(self.operations) else "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 1}}, {"type": "message_stop"}])
            return events
        events = [{"type": "response.created", "response": {"id": "response_" + str(stage), "status": "in_progress"}}]
        if stage < len(self.operations):
            if self.operations[stage][0] != "Bash":
                patch = "*** Begin Patch\n*** Update File: edit.txt\n@@\n-before\n+after\n*** End Patch" if stage == 0 else "*** Begin Patch\n*** Add File: ../outside.txt\n+outside\n*** End Patch"
                if stage >= 2:
                    patch = "*** Begin Patch\n*** Update File: .git/config\n@@\n" + "".join(" " + line + "\n" for line in self.initial_config.splitlines()) + "+\n+# synthetic edit marker\n*** End Patch"
                item = {"type": "custom_tool_call", "call_id": call_id, "namespace": "functions", "name": "apply_patch", "input": patch}
            else:
                item = {"type": "function_call", "call_id": call_id, "namespace": "functions", "name": "exec_command", "arguments": json.dumps({"cmd": self.operations[stage][1]["command"], "workdir": str(self.cwd), "yield_time_ms": 1000, **{k: v for k, v in self.operations[stage][1].items() if k != "command"}})}
            events.append({"type": "response.output_item.done", "item": item})
        events.append({"type": "response.completed", "response": {"id": "response_" + str(stage), "status": "completed", "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}})
        return events


class WorkspaceProtocol:
    def __init__(self, process, provider):
        self.process, self.provider = process, provider
        self.responses = queue.Queue()
        self.requests, self.transcript = [], []
        self.lock = threading.Lock()
        threading.Thread(target=self.read, daemon=True).start()

    def send(self, value):
        with self.lock:
            self.process.stdin.write(json.dumps(value) + "\n")
            self.process.stdin.flush()

    def read(self):
        for line in self.process.stdout:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            self.transcript.append(value)
            if self.provider == "claude" and value.get("type") == "control_request":
                self.requests.append(value)
                request = value.get("request", {})
                self.send({"type": "control_response", "response": {"subtype": "success", "request_id": value["request_id"], "response": {"behavior": "allow", "updatedInput": request.get("input", {})}}})
            elif self.provider == "codex" and value.get("method") and value.get("id") is not None:
                self.requests.append(value)
                self.send({"id": value["id"], "result": {"decision": "accept"}})
            elif value.get("method") is None and value.get("id") is not None:
                self.responses.put(value)

    def response(self, number):
        while True:
            value = self.responses.get(timeout=30)
            if value.get("id") == number:
                if "error" in value:
                    raise RuntimeError(json.dumps(value["error"]))
                return value["result"]


def prepare(root):
    home, cwd = root / "home", root / "repo"
    home.mkdir()
    cwd.mkdir()
    (cwd / "edit.txt").write_text("before\n")
    for name in ["AGENTS.md", "CLAUDE.md"]:
        (cwd / name).write_text("DISCARD_REPOSITORY_SENTINEL: synthetic instruction marker.\n")
    env = os.environ.copy()
    env.update(HOME=str(home), CODEX_HOME=str(home), CLAUDE_CONFIG_DIR=str(home), GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1", GIT_OPTIONAL_LOCKS="0", GIT_AUTHOR_NAME="Synthetic", GIT_AUTHOR_EMAIL="synthetic@example.invalid", GIT_COMMITTER_NAME="Synthetic", GIT_COMMITTER_EMAIL="synthetic@example.invalid")
    for key in list(env):
        if key.startswith(("ANTHROPIC_", "OPENAI_", "CODEX_API", "CLAUDE_CODE_OAUTH")) or key in ["CLAUDECODE", "CLAUDE_CODE_PLUGIN_DIRS"]:
            env.pop(key, None)
    for args in [["init", "-q"], ["add", "."], ["-c", "user.name=Synthetic", "-c", "user.email=synthetic@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "Synthetic baseline"]]:
        subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", *args], cwd=cwd, env=env, check=True, capture_output=True)
    return home, cwd, env


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
    process.wait(timeout=10)


def report(execution, protocol, root, home, cwd):
    git_results = {
        "touch_git_marker_succeeded": (cwd / ".git/marker").exists(),
        "git_add_succeeded": bool(subprocess.check_output(["git", "diff", "--cached", "--name-only"], cwd=cwd, text=True)),
        "git_commit_succeeded": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=cwd, text=True).strip() != execution.initial_head,
        "edit_git_config_succeeded": (cwd / ".git/config").read_text() != execution.initial_config,
    }
    # A successful commit consumes the index and therefore also proves git add succeeded.
    git_results["git_add_succeeded"] |= git_results["git_commit_succeeded"]
    return {
        "main_checkout": execution.main_checkout,
        "git_protection": git_results,
        "protects_git_in_root": execution.finished.is_set() and not any(git_results.values()),
        "operations": execution.operations,
        "completed": execution.finished.is_set(), "model_tools": sorted(set.union(set(), *execution.tool_surfaces)),
        "repository_instructions_seen": execution.instructions_seen,
        "approval_requests": protocol.requests, "tool_results": execution.results,
        "files": {"edit.txt": (cwd / "edit.txt").read_text(), "inside.txt": (cwd / "inside.txt").exists(), "outside.txt": (root / "outside.txt").exists(), "home_marker": (home / "outside-marker").exists()},
    }


def scrub(value, root):
    return json.loads(json.dumps(value).replace(str(root), "/synthetic").replace(str(root.resolve()), "/synthetic"))


def save_transcript(provider, transcript):
    path = pathlib.Path(__file__).resolve().parents[2] / "fixtures" / (provider + "-workspace-turn.jsonl")
    path.write_text("".join(json.dumps(value) + "\n" for value in transcript))


def command_marker(protocol):
    params = [r["params"] for r in protocol.requests if r.get("method") == "item/commandExecution/requestApproval"]
    inside = [p for p in params if "inside-comparison-" in p.get("command", "")]
    escalation = [p for p in params if "escalation-" in p.get("command", "")]
    differing = {}
    for key in sorted(set().union(*(p.keys() for p in inside + escalation))):
        left = [p.get(key) for p in inside]
        right = [p.get(key) for p in escalation]
        if left != right:
            differing[key] = {"in_sandbox": left, "escalation": right}
    # IDs, command text, and suggested rules are request data, not sandbox guarantees.
    return {"codex_in_sandbox_marker": "none", "inside_approval_count": len(inside), "escalation_approval_count": len(escalation), "differing_params": differing}
