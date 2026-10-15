"""Build-time dependency preparation boundary tests; no interpreter fallback."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("prepare_python_deps", ROOT / "scripts/prepare-python-deps.py")
preparation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(preparation)


class DependencyPreparationTests(unittest.TestCase):
    def test_compatible_and_incompatible_tags(self):
        from pip._vendor.packaging.tags import Tag
        for tag in [Tag("py3", "none", "any"), Tag("cp314", "cp314", "win_amd64"), Tag("cp38", "abi3", "win_amd64")]:
            self.assertTrue(preparation.check_tag(tag), tag)
        for tag in [Tag("cp314t", "cp314t", "win_amd64"), Tag("cp313", "cp313", "win_amd64"), Tag("cp314", "cp314", "win_arm64"), Tag("cp315", "abi3", "win_amd64")]:
            self.assertFalse(preparation.check_tag(tag), tag)

    def test_missing_transitive_dependency_and_conflict(self):
        Requirement = preparation.Requirement
        with self.assertRaisesRegex(ValueError, "Missing"):
            preparation.dependency_closure([{"name": "sample", "version": "1", "requires": ["child>=2"]}], [Requirement("sample==1")])
        with self.assertRaisesRegex(ValueError, "Conflicting"):
            preparation.dependency_closure([{"name": "sample", "version": "1"}, {"name": "sample", "version": "2"}], [Requirement("sample")])

    def test_target_markers_and_extras(self):
        packages = [{"name": "sample", "version": "1", "requires": ["child>=2; extra == 'math'", "ignored; sys_platform == 'linux'"]},
                    {"name": "child", "version": "2", "requires": []}]
        preparation.dependency_closure(packages, [preparation.Requirement("sample[math]")])
        with self.assertRaisesRegex(ValueError, "Missing"):
            preparation.dependency_closure(packages[:1], [preparation.Requirement("sample[math]")])

    def test_unsafe_or_site_dependent_wheels_rejected(self):
        for filename in ["../escape.py", "pkg/setup.pth", "sample.data/scripts/launch.exe", "C:/absolute.py"]:
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                wheel = root / "sample.whl"
                with zipfile.ZipFile(wheel, "w") as archive:
                    archive.writestr(filename, "invalid")
                with self.assertRaises(ValueError):
                    preparation.extract_wheel(wheel, root / "dependencies", set())

    def test_dist_info_license_and_native_files_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wheel = root / "sample.whl"
            paths = ["sample/__init__.py", "sample/native.pyd", "sample.libs/helper.dll", "sample.dist-info/licenses/LICENSE"]
            with zipfile.ZipFile(wheel, "w") as archive:
                for path in paths:
                    archive.writestr(path, path)
            preparation.extract_wheel(wheel, root / "dependencies", set())
            for path in paths:
                self.assertEqual((root / "dependencies" / path).read_text(), path)


if __name__ == "__main__":
    unittest.main()
