"""Probe Codex workspace controls against a synthetic local model endpoint."""
import argparse
import json
import os
import pathlib
import re
import subprocess
import tempfile
from codex_conversation import require_controls
from on_request_checks import Listener, all_passed, OnRequestExecution, collect, evaluate, linked_worktree, print_table
from workspace_execution import WorkspaceExecution, WorkspaceProtocol, command_marker, prepare, report, save_transcript, scrub, stop


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex", default=os.environ.get("DISCARD_CODEX_BIN", "codex"))
    parser.add_argument("--codex-home", type=pathlib.Path, default=pathlib.Path(os.environ.get("CODEX_HOME", str(pathlib.Path.home() / ".codex"))))
    parser.add_argument("--main-checkout", action="store_true")
    parser.add_argument("--on-request", action="store_true", help="Autorun spike (spec decision 12): on-request approval in a linked worktree")
    options = parser.parse_args()
    version = subprocess.check_output([options.codex, "--version"], text=True, timeout=10).strip()
    assert version == "codex-cli 0.166.0", version
    cache = json.loads((options.codex_home / "models_cache.json").read_text())
    cache_version = cache.get("client_version") # Metadata only; the running CLI stays pinned.
    models = cache["models"]
    selected = models[0]["slug"]
    for model in models:
        model.update(experimental_supported_tools=[], multi_agent_version="disabled", node_repl_disabled=True, tool_mode="direct")
    source = (pathlib.Path(__file__).resolve().parents[4] / "crates/discard-providers/src/conversation.rs").read_text()
    features = re.findall(r'"([^"]+)"', re.search(r"for name in \[(.*?)\] \{", source, re.S)[1])
    controls = {"features": {name: False for name in features}, "web_search": "disabled", "agents": {"enabled": False}, "memories": {"use_memories": False, "dedicated_tools": False, "generate_memories": False}, "cloud": {"skills": {"enabled": False}}, "skills": {"include_instructions": False, "bundled": {"enabled": False}}, "project_doc_max_bytes": 0, "tools": {"experimental_request_user_input": {"enabled": False}, "update_plan": {"enabled": False}}, "sandbox_workspace_write": {"network_access": False, "writable_roots": []}}
    controls["features"].update(shell_tool=True, skip_host_skill_discovery=True)
    with tempfile.TemporaryDirectory(prefix="discard-codex-workspace-") as temporary:
        root = pathlib.Path(temporary).resolve()
        home, cwd, env = prepare(root)
        main, listener, edit_baseline, index_before = cwd, None, None, None
        if options.on_request:
            # The shell must not reach the main checkout through a writable system temporary directory.
            (home / "tmp").mkdir()
            env["TMPDIR"] = str(home / "tmp")
            cwd = linked_worktree(root, main, env)
            listener = Listener()
            edit_baseline = (main / ".git/config").read_text()
            index_before = subprocess.run(["git", "diff", "--cached", "--name-only"], cwd=cwd, capture_output=True, text=True).stdout.strip()
        catalog = root / "models.json"
        catalog.write_text(json.dumps({"models": models}))
        controls["model_catalog_json"] = str(catalog)
        with (OnRequestExecution(cwd, main, listener.port) if options.on_request else WorkspaceExecution("codex", cwd, options.main_checkout)) as execution:
            config = dict(controls)
            config.update(model_provider="discard_synthetic", model_providers={"discard_synthetic": {"name": "Synthetic test", "base_url": f"http://127.0.0.1:{execution.port}/v1", "wire_api": "responses", "requires_openai_auth": False}})
            def toml_value(value):
                if isinstance(value, dict):
                    return "{" + ", ".join(json.dumps(k) + " = " + toml_value(v) for k, v in value.items()) + "}"
                return json.dumps(value)
            args = [options.codex, "app-server"]
            for key, value in config.items():
                args.extend(["-c", key + "=" + toml_value(value)])
            args.extend(["--listen", "stdio://"])
            with (root / "stderr").open("w+") as stderr:
                process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, start_new_session=True)
                protocol = WorkspaceProtocol(process, "codex")
                if options.on_request:
                    protocol.decision = "decline"  # Observe requests; never grant one.
                    protocol.observer = lambda message: execution.observe(message, cwd / "edit.txt")
                try:
                    protocol.send({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "discard_synthetic", "version": "1"}}})
                    protocol.response(1)
                    protocol.send({"method": "initialized"})
                    protocol.send({"id": 2, "method": "config/read", "params": {"cwd": str(cwd), "includeLayers": True}})
                    reported = protocol.response(2)
                    effective = reported["config"]
                    effective["tools"] = {}
                    for layer in reversed(reported.get("layers", [])):
                        if layer.get("disabledReason") is None:
                            effective["tools"].update(layer["config"].get("tools", {}))
                    require_controls(effective, controls)
                    protocol.send({"id": 3, "method": "thread/start", "params": {"cwd": str(cwd), "model": selected, "allowProviderModelFallback": False, "approvalPolicy": "on-request" if options.on_request else "untrusted", "sandbox": "workspace-write", "ephemeral": True, "config": controls}})
                    thread = protocol.response(3)["thread"]["id"]
                    protocol.send({"id": 4, "method": "turn/start", "params": {"threadId": thread, "input": [{"type": "text", "text": "Run the synthetic workspace sequence."}]}})
                    protocol.response(4)
                    execution.finished.wait(110 if options.on_request else 55)
                finally:
                    stop(process)
                if options.on_request:
                    observed = collect(execution, protocol, listener, root, edit_baseline, index_before)
                    rows = evaluate(observed)
                    summary = {"provider": "codex", "version": version, "approval_policy": "on-request", "sandbox": "workspace-write", "linked_worktree": True, "completed": execution.finished.is_set(), "approval_requests": protocol.requests, "observed": observed}
                    # Keep the turn messages only. Startup replies carry host details.
                    turn_messages = [m for m in protocol.transcript if str(m.get("method", "")).split("/")[0] in ("item", "turn", "serverRequest")]
                    save_transcript("codex", scrub(turn_messages, root), "autorun-turn")
                    print(json.dumps(scrub(summary, root), indent=2))
                    print_table(scrub(rows, root))
                    listener.close()
                    assert execution.finished.is_set(), "Synthetic sequence did not finish"
                    assert all_passed(rows), "An on-request autorun check failed. Codex autorun must stay unsupported."
                    return
                result = report(execution, protocol, root, home, cwd)
                result.update(command_marker(protocol))
                result["codex_protects_git_in_root"] = result["protects_git_in_root"]
                result.update(provider="codex", version=version, cache_version=cache_version, sandbox_workspace_write=effective.get("sandbox_workspace_write"), shell_type=models[0].get("shell_type"), apply_patch_tool_type=models[0].get("apply_patch_tool_type"))
                stderr.seek(0)
                result["stderr"] = stderr.read()[-2000:]
            if not options.main_checkout:
                save_transcript("codex", scrub(protocol.transcript, root))
            print(json.dumps(scrub(result, root), indent=2))
            assert result["completed"], "Synthetic sequence did not finish"


if __name__ == "__main__":
    main()
