"""Control-check regressions. These tests do not start a provider or use a network."""
import pathlib
import subprocess
import sys
import unittest

from codex_conversation import require_controls


class RequiredControlsTests(unittest.TestCase):
    def test_matching_controls_pass(self):
        require_controls({"features": {"shell_tool": False}}, {"features": {"shell_tool": False}})

    def test_mismatch_exits_with_failure_before_continuation(self):
        script = (
            "from codex_conversation import require_controls; "
            "require_controls({'memories': {'use_memories': True}}, "
            "{'memories': {'use_memories': False}}); "
            "print('THREAD_START_MUST_NOT_RUN')"
        )
        result = subprocess.run(
            [sys.executable, "-c", script],
            cwd=pathlib.Path(__file__).resolve().parent,
            capture_output=True,
            text=True,
            timeout=10,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("memories.use_memories", result.stderr)
        self.assertNotIn("THREAD_START_MUST_NOT_RUN", result.stdout)

    def test_missing_nested_control_fails(self):
        with self.assertRaisesRegex(AssertionError, "skills.include_instructions"):
            require_controls({}, {"skills": {"include_instructions": False}})


if __name__ == "__main__":
    unittest.main()
