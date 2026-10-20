import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("platforms", pathlib.Path(__file__).with_name("check-platforms.py"))
platforms = importlib.util.module_from_spec(spec)
spec.loader.exec_module(platforms)


def index(*architectures):
    return {"manifests": [{"platform": {"os": "linux", "architecture": arch}} for arch in architectures]}


class SupportedPlatforms(unittest.TestCase):
    def test_accepts_both_architectures_and_ignores_attestations(self):
        platforms.validate(index("amd64", "arm64", "unknown"))

    def test_rejects_missing_duplicate_and_unexpected_platforms(self):
        for architectures in [("amd64",), ("arm64",), ("amd64", "amd64"), ("amd64", "arm64", "arm")]:
            with self.subTest(architectures=architectures), self.assertRaises(ValueError):
                platforms.validate(index(*architectures))


if __name__ == "__main__":
    unittest.main()
