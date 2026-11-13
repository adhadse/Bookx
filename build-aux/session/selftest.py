#!/usr/bin/python3
"""Tests of the pure parts of the session harness (run first by session-tests.sh)."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    import harness
except ImportError as e:  # no PIL / PyGObject: session-tests.sh skips itself before it gets here
    print("skipping harness self-test:", e)
    sys.exit(0)


class RegistryDetection(unittest.TestCase):
    def test_the_failure_seen_in_a_real_run(self):
        log = "Gtk-WARNING **: Could not activate remote peer 'org.a11y.atspi.Registry': GDBus.Error:Timeout was reached"
        self.assertTrue(harness.registry_failure_in(log))

    def test_registry_not_activatable(self):
        self.assertTrue(harness.registry_failure_in(
            "The name org.a11y.atspi.Registry was not provided by any .service files"))

    def test_normal_logs_are_not_a_failure(self):
        self.assertFalse(harness.registry_failure_in(""))
        self.assertFalse(harness.registry_failure_in("Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display"))
        self.assertFalse(harness.registry_failure_in("atspi: registered org.a11y.atspi.Registry"))


if __name__ == "__main__":
    unittest.main()
