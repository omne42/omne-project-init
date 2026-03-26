import unittest

from __PY_PACKAGE__ import project_name


class ProjectNameTest(unittest.TestCase):
    def test_project_name(self) -> None:
        self.assertEqual(project_name(), "__REPO_NAME__")


if __name__ == "__main__":
    unittest.main()

