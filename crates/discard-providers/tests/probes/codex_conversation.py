"""Opt-in Codex tool exposure and execution check using local synthetic endpoints."""
import argparse
import json
import os
import pathlib
import re
import signal
import subprocess
import tempfile

from memory_execution import MemoryExecution, ProbeProcess, TOOLS


def require_controls(actual, expected):
    """Fail before thread creation if any required effective control differs."""
    mismatches = []

    def compare(found, required, path):
        if isinstance(required, dict):
            found = found if isinstance(found, dict) else {}
            for key, value in required.items():
                compare(found.get(key), value, path + [key])
        elif found != required:
            mismatches.append(".".join(path))

    compare(actual, expected, [])
    if mismatches:
        raise AssertionError("Required controls differ: " + ", ".join(mismatches))


def main():
    parser = argparse.ArgumentParser(description="Check Codex conversation memory execution with local endpoints.")
    parser.add_argument("--codex", default=os.environ.get("DISCARD_CODEX_BIN", "codex"), help="Codex CLI path (default: PATH)")
    parser.add_argument("--codex-home", type=pathlib.Path, default=pathlib.Path(os.environ.get("CODEX_HOME", str(pathlib.Path.home() / ".codex"))), help="Source home for model metadata only; authentication is not copied")
    options = parser.parse_args()
    assert subprocess.check_output([options.codex, "--version"], text=True, timeout=10).strip() == "codex-cli 0.166.0"
    repo = pathlib.Path(__file__).resolve().parents[4]
    source = (repo / "crates/discard-providers/src/conversation.rs").read_text()
    features = re.search(r"for name in \[(.*?)\] \{", source, re.S)[1]
    controls = {
        "web_search": "disabled", "agents": {"enabled": False},
        "memories": {"use_memories": False, "dedicated_tools": False, "generate_memories": False},
        "cloud": {"skills": {"enabled": False}}, "skills": {"include_instructions": False, "bundled": {"enabled": False}},
        "project_doc_max_bytes": 0, "tools": {"experimental_request_user_input": {"enabled": False}, "update_plan": {"enabled": False}},
        "features": {name: False for name in re.findall(r'"([^"]+)"', features)},
        "mcp_servers": {"inherited_synthetic": {"enabled": False}},
    }
    controls["features"]["skip_host_skill_discovery"] = True
    with MemoryExecution("codex") as execution, tempfile.TemporaryDirectory(prefix="discard-codex-execution-") as temporary:
        root = pathlib.Path(temporary)
        home, cwd = root / "home", root / "cwd"
        home.mkdir()
        cwd.mkdir()
        (home / "config.toml").write_text('[mcp_servers.inherited_synthetic]\nurl="http://127.0.0.1:' + str(execution.port) + '/forbidden"\n')
        cache = json.loads((options.codex_home / "models_cache.json").read_text())
        assert cache.get("client_version") == "0.166.0", "A model cache from Codex 0.166.0 is required"
        models = cache["models"]
        selected = models[0]["slug"]
        for model in models:
            model.update(apply_patch_tool_type=None, shell_type="disabled", experimental_supported_tools=[], multi_agent_version="disabled", node_repl_disabled=True, tool_mode="direct")
        catalog = root / "models.json"
        catalog.write_text(json.dumps({"models": models}))
        controls["model_catalog_json"] = str(catalog)

        def toml_value(value):
            if isinstance(value, dict):
                return "{" + ", ".join(json.dumps(key) + " = " + toml_value(item) for key, item in value.items()) + "}"
            return json.dumps(value)

        config = dict(controls)
        config.update(model_provider="discard_synthetic", model_providers={"discard_synthetic": {"name": "Synthetic test", "base_url": f"http://127.0.0.1:{execution.port}/v1", "wire_api": "responses", "requires_openai_auth": False}})
        args = [options.codex, "app-server"]
        for key, value in config.items():
            args.extend(["-c", key + "=" + toml_value(value)])
        args.extend(["--listen", "stdio://"])
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        for key in ["OPENAI_API_KEY", "CODEX_API_KEY"]:
            env.pop(key, None)
        process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
        protocol = ProbeProcess(process, "codex")
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
            thread_config = dict(controls)
            thread_config["mcp_servers"] = {
                "inherited_synthetic": {"enabled": False},
                "discard-memory": {"url": f"http://127.0.0.1:{execution.port}/mcp", "http_headers": {"Authorization": "Bearer synthetic"}, "enabled": True, "default_tools_approval_mode": "approve", "enabled_tools": TOOLS, "omit_tools_from": ["deferred", "code_mode"]},
            }
            protocol.send({"id": 3, "method": "thread/start", "params": {"cwd": str(cwd), "model": selected, "allowProviderModelFallback": False, "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": True, "config": thread_config}})
            thread = protocol.response(3)["thread"]["id"]
            protocol.send({"id": 4, "method": "turn/start", "params": {"threadId": thread, "input": [{"type": "text", "text": "Synthetic local memory test."}]}})
            protocol.response(4)
            execution.verify()
            assert not protocol.requests, "Unexpected provider approval request"
        finally:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


if __name__ == "__main__":
    main()
