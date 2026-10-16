"""Focused target-tag tests for the macOS Python dependency preparer."""
import runpy
import unittest
from pathlib import Path

from pip._vendor.packaging.tags import Tag

PREPARER = runpy.run_path(Path(__file__).with_name("prepare-python-macos.py"))
wheel_tags_compatible = PREPARER["wheel_tags_compatible"]


class MacWheelCompatibilityTests(unittest.TestCase):
    def test_rejects_wheel_with_newer_minimum_macos(self):
        tag = Tag("cp314", "cp314", "macosx_15_0_arm64")
        self.assertFalse(wheel_tags_compatible({tag}, "aarch64-apple-darwin"))

    def test_accepts_older_x86_64_platform_tag(self):
        tag = Tag("cp314", "cp314", "macosx_10_9_x86_64")
        self.assertTrue(wheel_tags_compatible({tag}, "x86_64-apple-darwin"))

    def test_rejects_native_wheel_for_other_architecture(self):
        tag = Tag("cp314", "cp314", "macosx_14_0_x86_64")
        self.assertFalse(wheel_tags_compatible({tag}, "aarch64-apple-darwin"))

    def test_universal2_tag_is_supported_on_both_hosts(self):
        tag = Tag("cp314", "cp314", "macosx_14_0_universal2")
        self.assertTrue(wheel_tags_compatible({tag}, "aarch64-apple-darwin"))
        self.assertTrue(wheel_tags_compatible({tag}, "x86_64-apple-darwin"))


if __name__ == "__main__":
    unittest.main()
