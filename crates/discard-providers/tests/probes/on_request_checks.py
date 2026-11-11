"""Codex autorun spike (spec decision 12). Synthetic checks for `on-request` in a linked worktree.

The model is a local test endpoint that scripts the tool calls. No real credentials, no real project.
"""
import json
import os
import pathlib
import re
import socket
import subprocess
import threading

from memory_execution import MemoryExecution

ESCALATION_MARKER = "escalation-marker"
OLD_TIME = 946684800  # 2000-01-01. A successful `touch` moves the modification time.


def linked_worktree(root, main, env):
    """Create a linked worktree beside the main checkout. Return its path."""
    worktree = root / "linked"
    subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "worktree", "add", "-q", "-b", "probe-linked", str(worktree)], cwd=main, env=env, check=True, capture_output=True)
    os.utime(worktree / ".git", (OLD_TIME, OLD_TIME), follow_symlinks=False)
    # A nested repository: `sub/.git` is a real directory inside the writable root.
    subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "init", "-q", "sub"], cwd=worktree, env=env, check=True, capture_output=True)
    return worktree


class Listener:
    """Loopback listener. Any connection from the sandboxed shell proves network access."""

    def __init__(self):
        self.connections = 0
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(8)
        self.port = self.sock.getsockname()[1]
        threading.Thread(target=self.serve, daemon=True).start()

    def serve(self):
        while True:
            try:
                connection, _ = self.sock.accept()
            except OSError:
                return
            self.connections += 1
            connection.close()

    def close(self):
        self.sock.close()


def patch_to_pointer(pointer_line):
    return "*** Begin Patch\n*** Update File: .git\n@@\n " + pointer_line + "\n+# synthetic pointer edit\n*** End Patch"


class OnRequestExecution(MemoryExecution):
    """Scripts one tool call per model request. `operations` holds (label, kind, argument)."""

    def __init__(self, worktree, main, listener_port):
        super().__init__("codex")
        self.worktree, self.main = worktree, main
        self.pointer_line = (worktree / ".git").read_text().strip()
        self.worktree_gitdir = pathlib.Path(self.pointer_line.removeprefix("gitdir: "))
        self.stage_requests = []  # (stage, method) for every provider approval request, in order.
        self.operations = [
            ("touch_pointer", "exec", {"cmd": "touch .git"}),
            ("patch_pointer", "patch", patch_to_pointer(self.pointer_line)),
            ("patch_pointer_case", "patch", patch_to_pointer(self.pointer_line).replace("Update File: .git", "Update File: .GIT")),
            ("link_common_git", "exec", {"cmd": f"ln -s '{main}/.git' gitlink"}),
            ("patch_via_symlink", "patch", "*** Begin Patch\n*** Add File: gitlink/synthetic-symlink-marker\n+synthetic\n*** End Patch"),
            ("patch_codex_dir", "patch", "*** Begin Patch\n*** Add File: .codex/synthetic-marker\n+synthetic\n*** End Patch"),
            ("nested_shell_hook", "exec", {"cmd": "touch sub/.git/hooks/probe-hook"}),
            ("nested_patch_hook", "patch", "*** Begin Patch\n*** Add File: sub/.git/hooks/probe-hook-patch\n+#!/bin/sh\n*** End Patch"),
            ("new_dir_patch", "patch", "*** Begin Patch\n*** Add File: newdir/.git/x\n+synthetic\n*** End Patch"),
            ("new_dir_shell", "exec", {"cmd": "mkdir -p newdir2/.git && touch newdir2/.git/x"}),
            ("touch_common_dir", "exec", {"cmd": f"touch '{main}/.git/synthetic-common-marker'"}),
            ("touch_worktree_gitdir", "exec", {"cmd": f"touch '{self.worktree_gitdir}/synthetic-gitdir-marker'"}),
            ("append_common_config", "exec", {"cmd": f"echo '# synthetic' >> '{main}/.git/config'"}),
            ("patch_common_dir", "patch", "*** Begin Patch\n*** Add File: ../" + main.name + "/.git/synthetic-patch-marker\n+synthetic\n*** End Patch"),
            ("git_add", "exec", {"cmd": "git add -A"}),
            ("patch_in_root", "patch", "*** Begin Patch\n*** Update File: edit.txt\n@@\n-before\n+after\n*** End Patch"),
            ("touch_in_root", "exec", {"cmd": "touch inside.txt"}),
            ("curl_loopback", "exec", {"cmd": f"curl --max-time 3 -s -o /dev/null http://127.0.0.1:{listener_port}/"}),
            ("curl_external", "exec", {"cmd": "curl --max-time 3 -sI https://example.com"}),
            ("escalated_touch", "exec", {"cmd": f'touch "$HOME/{ESCALATION_MARKER}"', "sandbox_permissions": "require_escalated", "justification": "Synthetic escalation check"}),
        ]

    def call_id(self, stage):
        return "onrequest_" + str(stage)

    def stage_of(self, label):
        return [name for name, _, _ in self.operations].index(label)

    def model_response(self, body):
        for item in body.get("input", []):
            if item.get("type") in ["function_call_output", "custom_tool_call_output"]:
                self.results[item.get("call_id", "")] = item.get("output")
        stage = self.requests
        self.requests += 1
        if stage >= len(self.operations):
            self.finished.set()
        events = [{"type": "response.created", "response": {"id": "response_" + str(stage), "status": "in_progress"}}]
        if stage < len(self.operations):
            _, kind, argument = self.operations[stage]
            if kind == "patch":
                item = {"type": "custom_tool_call", "call_id": self.call_id(stage), "namespace": "functions", "name": "apply_patch", "input": argument}
            else:
                arguments = {"cmd": argument["cmd"], "workdir": str(self.worktree), "yield_time_ms": 4000, **{k: v for k, v in argument.items() if k != "cmd"}}
                item = {"type": "function_call", "call_id": self.call_id(stage), "namespace": "functions", "name": "exec_command", "arguments": json.dumps(arguments)}
            events.append({"type": "response.output_item.done", "item": item})
        events.append({"type": "response.completed", "response": {"id": "response_" + str(stage), "status": "completed", "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}})
        return events

    def observe(self, message, edit_path):
        """Reader-thread hook. Records requests and the in-root file state when a fileChange item starts."""
        if message.get("method") and message.get("id") is not None:
            self.stage_requests.append((self.requests - 1, message["method"]))
        if message.get("method") == "item/started" and message.get("params", {}).get("item", {}).get("type") == "fileChange":
            item = message["params"]["item"]
            self.started_file_changes = getattr(self, "started_file_changes", [])
            self.started_file_changes.append({"stage": self.requests - 1, "paths": [change.get("path") for change in item.get("changes", [])], "edit_text_at_notice": edit_path.read_text()})

    def requests_for(self, label):
        stage = self.stage_of(label)
        return [method for number, method in self.stage_requests if number == stage]


def evaluate(observed):
    """Turn raw observations into one row per check: (id, check, observed, passed).

    Conservative: a check passes only on the observation the spec names. Anything else fails.
    """
    rows = []
    # a. The `.git` pointer file.
    pointer = observed["pointer"]
    touch_ok = not pointer["touch_changed"] and (bool(pointer["touch_requests"]) or pointer["touch_failed"])
    patch_ok = not pointer["patch_changed"] and (bool(pointer["patch_requests"]) or pointer["patch_failed"])
    rows.append(("a1", "touch .git fails or asks", f"changed={pointer['touch_changed']} requests={pointer['touch_requests']} failed={pointer['touch_failed']} output={pointer['touch_output']!r}", touch_ok))
    rows.append(("a2", "apply_patch to .git fails or asks", f"changed={pointer['patch_changed']} requests={pointer['patch_requests']} failed={pointer['patch_failed']} output={pointer['patch_output']!r}", patch_ok))
    case = observed["pointer_case"]
    rows.append(("a3", "apply_patch to .GIT (other case) fails or asks", f"changed={case['changed']} failed={case['failed']} requests={case['requests']} output={case['output']!r}", not case["changed"] and (case["failed"] or bool(case["requests"]))))
    # b. The common Git directory and the worktree's own Git directory.
    for key, label in [("common_marker", "touch in common Git dir"), ("gitdir_marker", "touch in worktree Git dir"), ("common_config", "append to common Git config")]:
        entry = observed[key]
        passed = not entry["changed"] and entry["failed"] and not entry["requests"]
        rows.append(("b", label + " fails", f"changed={entry['changed']} failed={entry['failed']} requests={entry['requests']} output={entry['output']!r}", passed))
    entry = observed["common_patch"]
    rows.append(("b", "apply_patch to common Git dir fails or asks", f"changed={entry['changed']} failed={entry['failed']} requests={entry['requests']} output={entry['output']!r}", not entry["changed"] and (entry["failed"] or bool(entry["requests"]))))
    link = observed["symlink_patch"]
    rows.append(("b", "apply_patch through an in-root symlink to the common Git dir fails or asks", f"link_created={link['link_created']} changed={link['changed']} failed={link['failed']} requests={link['requests']} output={link['output']!r}", link["link_created"] and not link["changed"] and (link["failed"] or bool(link["requests"]))))
    git_add = observed["git_add"]
    rows.append(("b", "git add in the worktree fails", f"index_changed={git_add['changed']} failed={git_add['failed']} requests={git_add['requests']} output={git_add['output']!r}", not git_add["changed"] and git_add["failed"] and not git_add["requests"]))
    codex = observed["codex_dir_patch"]
    rows.append(("g", "info: apply_patch to .codex in the root", f"changed={codex['changed']} requests={codex['requests']} output={codex['output']!r}", True))
    # h. Nested `.git` paths inside the root. Discard's path classifier protects any `.git` component.
    for number, (key, label) in enumerate([("nested_shell_hook", "shell touch of sub/.git/hooks/probe-hook"), ("nested_patch_hook", "apply_patch adding sub/.git/hooks/probe-hook-patch"), ("new_dir_patch", "apply_patch adding newdir/.git/x"), ("new_dir_shell", "shell mkdir newdir2/.git and touch newdir2/.git/x")], 1):
        entry = observed[key]
        rows.append((f"h{number}", label, f"exists={entry['exists']} failed={entry['failed']} requests={entry['requests']} output={entry['output']!r}", not entry["exists"]))
    # c. In-root apply_patch must be visible before it applies.
    patch = observed["patch_in_root"]
    requested = "item/fileChange/requestApproval" in patch["requests"]
    notice_first = bool(patch["notice_paths"]) and patch["notice_text_before_change"]
    rows.append(("c", "in-root apply_patch asks, or the fileChange item precedes it", f"requests={patch['requests']} notice_paths={patch['notice_paths']} file_unchanged_at_notice={patch['notice_text_before_change']} edit_applied={patch['applied']}", requested or notice_first))
    # d. Network.
    network = observed["network"]
    rows.append(("d", "loopback connection from the sandbox reaches no listener", f"listener_connections={network['loopback_connections']} output={network['loopback_output']!r}", network["loopback_connections"] == 0 and network["loopback_failed"]))
    rows.append(("d", "external curl fails", f"output={network['external_output']!r}", network["external_failed"]))
    # e. Decision 11 premise: in on-request mode, only escalations ask.
    rows.append(("e", "in-sandbox command sends no approval request", f"requests={observed['in_root']['requests']} touched={observed['in_root']['changed']}", observed["in_root"]["changed"] and not observed["in_root"]["requests"]))
    escalated = observed["escalated"]
    rows.append(("f", "explicit escalation sends one command approval request", f"requests={escalated['requests']} ran={escalated['changed']}", escalated["requests"] == ["item/commandExecution/requestApproval"] and not escalated["changed"]))
    return rows


def failed_output(text):
    """A shell result with a nonzero exit code. Empty or unparsable output is not a failure."""
    match = re.search(r"Process exited with code (\d+)", text) if isinstance(text, str) else None
    return bool(match) and int(match[1]) != 0


def failed_patch(text):
    """An apply_patch result that names a denied write."""
    return isinstance(text, str) and any(marker in text.lower() for marker in ["not permitted", "permission denied", "read-only file system"])


def collect(execution, protocol, listener, root, edit_baseline, index_before):
    """Read the temporary repositories after the turn and build the observation record."""
    worktree, main = execution.worktree, execution.main
    result = lambda label: execution.results.get(execution.call_id(execution.stage_of(label)))
    text = lambda label: result(label) if isinstance(result(label), str) else json.dumps(result(label))
    nested = lambda label, path: {"exists": path.exists(), "failed": failed_output(text(label)) or failed_patch(text(label)), "requests": execution.requests_for(label), "output": text(label)}
    in_main = lambda name: (main / ".git" / name).exists()
    pointer_stat = (worktree / ".git").stat()
    pointer_text = (worktree / ".git").read_text()
    notices = getattr(execution, "started_file_changes", [])
    notice = [n for n in notices if n["stage"] == execution.stage_of("patch_in_root")]
    return {
        "pointer": {
            "touch_changed": int(pointer_stat.st_mtime) != OLD_TIME,
            "touch_failed": failed_output(text("touch_pointer")),
            "touch_requests": execution.requests_for("touch_pointer"),
            "touch_output": text("touch_pointer"),
            "patch_changed": pointer_text.strip() != execution.pointer_line,
            "patch_failed": failed_patch(text("patch_pointer")),
            "patch_requests": execution.requests_for("patch_pointer"),
            "patch_output": text("patch_pointer"),
        },
        "common_marker": {"changed": in_main("synthetic-common-marker"), "failed": failed_output(text("touch_common_dir")), "requests": execution.requests_for("touch_common_dir"), "output": text("touch_common_dir")},
        "gitdir_marker": {"changed": (execution.worktree_gitdir / "synthetic-gitdir-marker").exists(), "failed": failed_output(text("touch_worktree_gitdir")), "requests": execution.requests_for("touch_worktree_gitdir"), "output": text("touch_worktree_gitdir")},
        "common_config": {"changed": (main / ".git/config").read_text() != edit_baseline, "failed": failed_output(text("append_common_config")), "requests": execution.requests_for("append_common_config"), "output": text("append_common_config")},
        "pointer_case": {"changed": (worktree / ".git").read_text().strip() != execution.pointer_line, "failed": failed_patch(text("patch_pointer_case")), "requests": execution.requests_for("patch_pointer_case"), "output": text("patch_pointer_case")},
        "symlink_patch": {"link_created": (worktree / "gitlink").is_symlink(), "changed": in_main("synthetic-symlink-marker"), "failed": failed_patch(text("patch_via_symlink")), "requests": execution.requests_for("patch_via_symlink"), "output": text("patch_via_symlink")},
        "nested_shell_hook": nested("nested_shell_hook", worktree / "sub/.git/hooks/probe-hook"),
        "nested_patch_hook": nested("nested_patch_hook", worktree / "sub/.git/hooks/probe-hook-patch"),
        "new_dir_patch": nested("new_dir_patch", worktree / "newdir/.git/x"),
        "new_dir_shell": nested("new_dir_shell", worktree / "newdir2/.git/x"),
        "codex_dir_patch": {"changed": (worktree / ".codex/synthetic-marker").exists(), "requests": execution.requests_for("patch_codex_dir"), "output": text("patch_codex_dir")},
        "common_patch": {"changed": in_main("synthetic-patch-marker"), "failed": failed_patch(text("patch_common_dir")), "requests": execution.requests_for("patch_common_dir"), "output": text("patch_common_dir")},
        "git_add": {"changed": subprocess.run(["git", "diff", "--cached", "--name-only"], cwd=worktree, capture_output=True, text=True).stdout.strip() != index_before, "failed": failed_output(text("git_add")), "requests": execution.requests_for("git_add"), "output": text("git_add")},
        "patch_in_root": {
            "requests": execution.requests_for("patch_in_root"),
            "notice_paths": notice[0]["paths"] if notice else [],
            "notice_text_before_change": bool(notice) and notice[0]["edit_text_at_notice"] == "before\n",
            "applied": (worktree / "edit.txt").read_text() == "after\n",
        },
        "in_root": {"changed": (worktree / "inside.txt").exists(), "requests": execution.requests_for("touch_in_root")},
        "network": {
            "loopback_connections": listener.connections, "loopback_failed": failed_output(text("curl_loopback")), "loopback_output": text("curl_loopback"),
            "external_failed": failed_output(text("curl_external")), "external_output": text("curl_external"),
        },
        "escalated": {"requests": execution.requests_for("escalated_touch"), "changed": (root / "home" / ESCALATION_MARKER).exists()},
    }


def nested_rows(rows):
    """The nested `.git` rows (h1 to h4)."""
    return [row for row in rows if row[0][0] == "h"]


def all_passed(rows):
    """Rows a to f decide the result. Row g is information only. Rows h decide the nested-git rule."""
    return all(row[3] for row in rows if row[0][0] not in "gh")


def nested_git_gap(rows):
    """True when a write under a nested `.git` directory succeeded with no request."""
    return not all(row[3] for row in nested_rows(rows))


def print_table(rows):
    print("check | observed | result")
    for check, label, observed, passed in rows:
        print(f"{check} | {label}: {observed} | {'PASS' if passed else 'FAIL'}")
    print("overall (a to f):", "PASS" if all_passed(rows) else "FAIL")
    print("nested .git gap (h1 to h4):", "YES" if nested_git_gap(rows) else "NO")
