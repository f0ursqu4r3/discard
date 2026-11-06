"""Opt-in Claude tool exposure and execution check using local synthetic endpoints."""
import argparse
import json
import os
import pathlib
import signal
import subprocess
import tempfile

from memory_execution import MemoryExecution, ProbeProcess, TOOLS


def main():
    parser = argparse.ArgumentParser(description="Check Claude conversation memory execution with local endpoints.")
    parser.add_argument("--claude", default=os.environ.get("DISCARD_CLAUDE_BIN", "claude"), help="Claude CLI path (default: PATH)")
    options = parser.parse_args()
    assert subprocess.check_output([options.claude, "--version"], text=True, timeout=10).startswith("2.1.318 ")
    with MemoryExecution("claude") as execution, tempfile.TemporaryDirectory(prefix="discard-claude-execution-") as temporary:
        root = pathlib.Path(temporary)
        config, cwd = root / "config", root / "cwd"
        config.mkdir()
        cwd.mkdir()
        mcp = root / "memory.json"
        mcp.write_text(json.dumps({"mcpServers": {"discard-memory": {"type": "http", "url": f"http://127.0.0.1:{execution.port}/mcp", "headers": {"Authorization": "Bearer synthetic"}}}}))
        env = os.environ.copy()
        env.update(CLAUDE_CONFIG_DIR=str(config), ANTHROPIC_API_KEY="synthetic", ANTHROPIC_BASE_URL=f"http://127.0.0.1:{execution.port}", CLAUDE_CODE_DISABLE_AUTO_MEMORY="1", CLAUDE_CODE_DISABLE_CLAUDE_MDS="1")
        for key in ["CLAUDE_CODE_PLUGIN_DIRS", "CLAUDECODE", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"]:
            env.pop(key, None)
        args = [
            options.claude, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--permission-mode", "manual", "--permission-prompts", "none", "--tools", "", "--strict-mcp-config", "--mcp-config", str(mcp),
            "--disable-slash-commands", "--restricted", "--system-prompt-snapshot", "off", "--model", "claude-sonnet-4-6",
            "--allowedTools", ",".join("mcp__discard-memory__" + name for name in TOOLS),
            "--settings", json.dumps({"disableAllHooks": True, "autoMemoryEnabled": False, "env": {"CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1", "CLAUDE_CODE_PLUGIN_DIRS": ""}, "pluginConfigs": {"cc-plugin-agents-md@builtin": {"options": {"instructionFiles": "managed-only"}}}}),
        ]
        process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
        protocol = ProbeProcess(process, "claude")
        try:
            protocol.send({"type": "user", "message": {"role": "user", "content": "Synthetic local memory test."}})
            execution.verify()
            assert not protocol.requests, "Unexpected provider approval request"
        finally:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


if __name__ == "__main__":
    main()
