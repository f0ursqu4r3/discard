"""The execution probe must reject advertised-only or incompletely applied calls."""
import contextlib
import io
import unittest

from memory_execution import EXPECTED_RESULTS, MemoryExecution, OPERATIONS, TOOLS


def completed_fixture():
    fixture = MemoryExecution("claude")
    for name, arguments in OPERATIONS:
        fixture.mcp_result("tools/call", {"name": name, "arguments": arguments})
    fixture.results = {"synthetic_" + str(index): value for index, value in enumerate(EXPECTED_RESULTS)}
    fixture.tool_surfaces = [{"mcp__discard-memory__" + name for name in TOOLS}]
    fixture.finished.set()
    return fixture


class MemoryExecutionTests(unittest.TestCase):
    def test_completed_calls_and_pending_proposal_pass(self):
        with contextlib.redirect_stdout(io.StringIO()):
            completed_fixture().verify(timeout=0)

    def test_tool_exposure_without_calls_fails(self):
        fixture = completed_fixture()
        fixture.receipts = []
        with self.assertRaisesRegex(AssertionError, "did not execute"):
            fixture.verify(timeout=0)

    def test_missing_result_delivery_fails(self):
        fixture = completed_fixture()
        del fixture.results["synthetic_1"]
        with self.assertRaisesRegex(AssertionError, "did not reach the model"):
            fixture.verify(timeout=0)

    def test_shared_write_before_approval_fails(self):
        fixture = completed_fixture()
        fixture.shared["items/unapproved.md"] = "Unexpected shared write"
        with self.assertRaisesRegex(AssertionError, "changed before approval"):
            fixture.verify(timeout=0)

    def test_applied_shared_proposal_fails(self):
        fixture = completed_fixture()
        fixture.proposals[0]["status"] = "applied"
        with self.assertRaisesRegex(AssertionError, "did not remain pending"):
            fixture.verify(timeout=0)


if __name__ == "__main__":
    unittest.main()
