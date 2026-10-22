import datetime
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("plan", pathlib.Path(__file__).with_name("plan-release.py"))
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)

TODAY = datetime.date(2026, 10, 22)
MINUTE = datetime.timedelta(minutes=1)


def pr(**fields):
    return {"isDraft": False, "mergeable": "MERGEABLE", "reviewDecision": "", "statusCheckRollup": [], **fields}


class NextVersion(unittest.TestCase):
    def test_starts_each_day_at_zero_and_ignores_legacy_tags(self):
        self.assertEqual(plan.next_version(["1.0", "v2.0.0", "v3.1.0", "v2026.10.21.4"], TODAY), "2026.10.22.0")

    def test_counts_numerically_within_the_day(self):
        tags = [f"v2026.10.22.{n}" for n in range(11)]
        self.assertEqual(plan.next_version(tags, TODAY), "2026.10.22.11")

    def test_refuses_to_go_backward(self):
        with self.assertRaises(ValueError):
            plan.next_version(["v2026.10.23.0"], TODAY)


class LandingSoon(unittest.TestCase):
    def test_green_or_running_prs_may_land(self):
        self.assertTrue(plan.may_land_soon(pr()))
        self.assertTrue(plan.may_land_soon(pr(statusCheckRollup=[{"status": "IN_PROGRESS", "conclusion": ""},
                                                                   {"conclusion": "SUCCESS"}, {"state": "PENDING"}])))

    def test_stuck_prs_do_not_hold_releases(self):
        for stuck in [pr(isDraft=True), pr(mergeable="CONFLICTING"), pr(reviewDecision="CHANGES_REQUESTED"),
                      pr(statusCheckRollup=[{"conclusion": "SUCCESS"}, {"conclusion": "FAILURE"}]),
                      pr(statusCheckRollup=[{"state": "ERROR"}])]:
            with self.subTest(stuck=stuck):
                self.assertFalse(plan.may_land_soon(stuck))


class Decide(unittest.TestCase):
    def decide(self, **overrides):
        return plan.decide(**{"forced": False, "changed": True, "ci": "success", "age": 20 * MINUTE, "landing": [], **overrides})[0]

    def test_releases_a_quiet_green_change(self):
        self.assertTrue(self.decide())

    def test_never_releases_unchanged_images(self):
        self.assertFalse(self.decide(changed=False))
        self.assertFalse(self.decide(changed=False, forced=True))

    def test_waits_for_green_ci_a_quiet_moment_and_pending_renovate_prs(self):
        self.assertFalse(self.decide(ci="failure"))
        self.assertFalse(self.decide(ci="in_progress", age=200 * MINUTE))
        self.assertFalse(self.decide(age=5 * MINUTE))
        self.assertFalse(self.decide(landing=[36]))

    def test_cap_overrides_pending_prs(self):
        self.assertTrue(self.decide(landing=[36], age=90 * MINUTE))

    def test_manual_requests_skip_waiting(self):
        self.assertTrue(self.decide(forced=True, ci="failure", age=MINUTE, landing=[36]))


if __name__ == "__main__":
    unittest.main()
