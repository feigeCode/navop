import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "update_gpui_component_revision.py"
SPEC = importlib.util.spec_from_file_location("update_gpui_component_revision", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UpdateGpuiComponentRevisionTests(unittest.TestCase):
    def test_updates_all_component_dependencies(self):
        revision = "a" * 40
        versions = {
            "gpui-component": "1.2.3",
            "gpui-component-assets": "1.2.4",
            "gpui-base": "1.2.5",
            "gpui-shell": "1.2.6",
            "gpui-component-shell": "1.2.7",
        }
        source = "".join(
            f'{name} = {{ package = "pkg", git = "https://github.com/feigeCode/gpui-component.git", rev = "old", version = "0.1.0" }}\n'
            for name in versions
        )

        updated = MODULE.update_manifest(source, revision, versions)

        for name, version in versions.items():
            line = next(line for line in updated.splitlines() if line.startswith(f"{name} = "))
            self.assertIn(f'rev = "{revision}"', line)
            self.assertIn(f'version = "{version}"', line)

    def test_missing_dependency_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "missing dependencies"):
            MODULE.update_manifest(
                "gpui-shell = {}\n",
                "a" * 40,
                {"gpui-shell": "1.0.0", "gpui-component": "1.0.0"},
            )

    def test_updates_json_and_rust_shell_versions(self):
        source = '"gpui_shell": "0.1.0"\ngpui_shell: "0.1.0".to_string()\n'

        updated = MODULE.update_shell_version(source, "0.2.0", "fixture")

        self.assertIn('"gpui_shell": "0.2.0"', updated)
        self.assertIn('gpui_shell: "0.2.0".to_string()', updated)

    def test_missing_shell_version_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "cannot find gpui-shell version"):
            MODULE.update_shell_version("nothing here", "0.2.0", "fixture")


if __name__ == "__main__":
    unittest.main()
