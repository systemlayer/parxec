import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

import cli


class GainTests(unittest.TestCase):
  def test_force_defaults_to_false(self) -> None:
    args = cli.build_parser().parse_args(["gain", "/music"])

    self.assertFalse(args.force)

  def test_parser_accepts_force(self) -> None:
    args = cli.build_parser().parse_args(["gain", "--force", "/music"])

    self.assertTrue(args.force)

  @patch("cli.subprocess.run")
  def test_gain_skips_existing_tags_by_default(self, mock_run) -> None:
    cli.handle_gain(Namespace(directory=Path("/music"), force=False))

    mock_run.assert_called_once_with(
        [
            "rsgain",
            "easy",
            "-p",
            "no_album",
            "-m",
            "MAX",
            "--skip-existing",
            "/music",
        ],
        check=True,
    )

  @patch("cli.subprocess.run")
  def test_force_processes_files_with_existing_tags(self, mock_run) -> None:
    cli.handle_gain(Namespace(directory=Path("/music"), force=True))

    mock_run.assert_called_once_with(
        [
            "rsgain",
            "easy",
            "-p",
            "no_album",
            "-m",
            "MAX",
            "/music",
        ],
        check=True,
    )


if __name__ == "__main__":
  unittest.main()
