"""Probe Claude workspace controls against a synthetic local model endpoint."""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
from workspace_execution import WorkspaceExecution, WorkspaceProtocol, prepare, report, save_transcript, scrub, stop


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--claude", default=os.environ.get("DISCARD_CLAUDE_BIN", "claude"))
    parser.add_argument("--main-checkout", action="store_true")
    options = parser.parse_args()
    version = subprocess.check_output([options.claude, "--version"], text=True, timeout=10).strip()
    assert version.startswith("2.1.318 "), version
    reports, transcript = [], []
    for restricted, protect_git in ([(True, True)] if options.main_checkout else [(True, False), (False, False), (True, True)]):
        with tempfile.TemporaryDirectory(prefix="discard-claude-workspace-") as temporary:
            root = pathlib.Path(temporary).resolve()
            home, cwd, env = prepare(root)
            with WorkspaceExecution("claude", cwd, options.main_checkout) as execution:
                env.update(ANTHROPIC_API_KEY="synthetic", ANTHROPIC_BASE_URL=f"http://127.0.0.1:{execution.port}", CLAUDE_CODE_DISABLE_AUTO_MEMORY="1", CLAUDE_CODE_DISABLE_CLAUDE_MDS="1")
                settings = {"disableAllHooks": True, "autoMemoryEnabled": False, "sandbox": {"enabled": True, "allowUnsandboxedCommands": False, "autoAllowBashIfSandboxed": False}, "env": {"CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1", "CLAUDE_CODE_PLUGIN_DIRS": "", "GIT_OPTIONAL_LOCKS": "0"}}
                if protect_git:
                    settings["sandbox"]["filesystem"] = {"denyWrite": [str(cwd / ".git")]}
                args = [options.claude, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-mode", "manual", "--permission-prompts", "host", "--permission-prompt-tool", "stdio", "--tools", "Read,Glob,Grep,Edit,Write,Bash", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--disable-slash-commands", "--system-prompt-snapshot", "off", "--model", "claude-sonnet-4-6", "--settings", json.dumps(settings)]
                if restricted:
                    args.append("--restricted")
                with (root / "stderr").open("w+") as stderr:
                    process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, start_new_session=True)
                    protocol = WorkspaceProtocol(process, "claude")
                    try:
                        protocol.send({"type": "user", "message": {"role": "user", "content": "Run the synthetic workspace sequence."}})
                        execution.finished.wait(55)
                    finally:
                        stop(process)
                    result = report(execution, protocol, root, home, cwd)
                    result["restricted"] = restricted
                    result["protect_git"] = protect_git
                    stderr.seek(0)
                    result["stderr"] = stderr.read()[-2000:]
                reports.append(scrub(result, root))
                transcript.extend(scrub(protocol.transcript, root))
    if not options.main_checkout:
        save_transcript("claude", transcript)
    print(json.dumps({"provider": "claude", "version": version, "claude_protects_git_in_root": all(run["protects_git_in_root"] for run in reports), "runs": reports}, indent=2))
    assert all(run["completed"] for run in reports), "Synthetic sequence did not finish"


if __name__ == "__main__":
    main()
