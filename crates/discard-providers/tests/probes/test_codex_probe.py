"""Control-check regressions. These tests do not start a provider or use a network."""
import pathlib
import subprocess
import sys
import unittest

from codex_conversation import require_controls
from on_request_checks import all_passed, evaluate, failed_output, failed_patch


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


def passing_observation():
    """What a safe Codex on-request sandbox looks like. Each test breaks one value."""
    denied = {"changed": False, "failed": True, "requests": [], "output": "Operation not permitted"}
    asked = {"changed": False, "failed": False, "requests": ["item/fileChange/requestApproval"], "output": "patch rejected by user"}
    return {
        "pointer": {"touch_changed": False, "touch_failed": True, "touch_requests": [], "touch_output": "", "patch_changed": False, "patch_failed": False, "patch_requests": ["item/fileChange/requestApproval"], "patch_output": ""},
        "pointer_case": dict(asked),
        "common_marker": dict(denied), "gitdir_marker": dict(denied), "common_config": dict(denied), "git_add": dict(denied),
        "common_patch": dict(asked), "symlink_patch": {**asked, "link_created": True}, "codex_dir_patch": {"changed": False, "requests": ["item/fileChange/requestApproval"], "output": ""},
        "patch_in_root": {"requests": [], "notice_paths": ["/root/edit.txt"], "notice_text_before_change": True, "applied": True},
        "in_root": {"changed": True, "requests": []},
        "network": {"loopback_connections": 0, "loopback_failed": True, "loopback_output": "", "external_failed": True, "external_output": ""},
        "escalated": {"requests": ["item/commandExecution/requestApproval"], "changed": False},
    }


class OnRequestEvaluationTests(unittest.TestCase):
    def failing(self, observed):
        return [row[0] for row in evaluate(observed) if not row[3]]

    def test_safe_observations_pass(self):
        self.assertTrue(all_passed(evaluate(passing_observation())))

    def test_a_writable_pointer_file_fails_check_a(self):
        observed = passing_observation()
        observed["pointer"].update(touch_changed=True, touch_failed=False)
        self.assertEqual(self.failing(observed), ["a1"])

    def test_a_silent_pointer_patch_fails_check_a(self):
        observed = passing_observation()
        observed["pointer"].update(patch_changed=True, patch_requests=[])
        self.assertEqual(self.failing(observed), ["a2"])

    def test_a_write_to_the_common_git_dir_fails_check_b(self):
        for key in ["common_marker", "gitdir_marker", "common_config", "common_patch", "symlink_patch"]:
            observed = passing_observation()
            observed[key].update(changed=True)
            self.assertEqual(self.failing(observed), ["b"], key)

    def test_a_symlink_patch_without_a_symlink_proves_nothing(self):
        observed = passing_observation()
        observed["symlink_patch"].update(link_created=False)
        self.assertEqual(self.failing(observed), ["b"])

    def test_an_in_root_patch_with_no_request_and_no_early_notice_fails_check_c(self):
        observed = passing_observation()
        observed["patch_in_root"].update(notice_paths=[])
        self.assertEqual(self.failing(observed), ["c"])
        observed = passing_observation()
        observed["patch_in_root"].update(notice_text_before_change=False)
        self.assertEqual(self.failing(observed), ["c"])

    def test_any_network_contact_fails_check_d(self):
        observed = passing_observation()
        observed["network"].update(loopback_connections=1)
        self.assertEqual(self.failing(observed), ["d"])
        observed = passing_observation()
        observed["network"].update(external_failed=False)
        self.assertEqual(self.failing(observed), ["d"])

    def test_a_command_that_asks_without_escalating_fails_the_premise(self):
        observed = passing_observation()
        observed["in_root"].update(requests=["item/commandExecution/requestApproval"])
        self.assertEqual(self.failing(observed), ["e"])

    def test_the_codex_dir_row_is_information_only(self):
        observed = passing_observation()
        observed["codex_dir_patch"].update(changed=True, requests=[])
        self.assertTrue(all_passed(evaluate(observed)))

    def test_failure_detection_needs_a_nonzero_exit_or_a_denial(self):
        self.assertTrue(failed_output("Process exited with code 1\nOutput:\n"))
        self.assertFalse(failed_output("Process exited with code 0\nOutput:\n"))
        self.assertFalse(failed_output("Output:\nerror in a file name\n"))
        self.assertFalse(failed_output(None))
        self.assertTrue(failed_patch("write: Operation not permitted"))
        self.assertFalse(failed_patch("patch rejected by user"))


if __name__ == "__main__":
    unittest.main()
