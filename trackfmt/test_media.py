import subprocess
import unittest
from unittest.mock import patch, MagicMock
import media


METAFLAC_OUTPUT = """
ALBUM=Test Album - テストアルバム
 RELEASETYPE = album
GROUPING=Test ☆ グルピング
ARTIST
REPLAYGAIN_TRACK_GAIN=-5.09 dB
REPLAYGAIN_TRACK_PEAK=0.891234
"""


class ReadFlacTagsTests(unittest.TestCase):
  @patch("media._run_metaflac")
  def test_parses_key_value_pairs_and_ignores_invalid_lines(self, mock_run_metaflac: MagicMock):
    mock_run_metaflac.return_value = subprocess.CompletedProcess(
        args=["metaflac"],
        returncode=0,
        stdout=METAFLAC_OUTPUT,
    )
    tags = media.read_flac_tags("audio.flac")
    self.assertEqual(
        tags,
        {
            "ALBUM": "Test Album - テストアルバム",
            "RELEASETYPE": "album",
            "GROUPING": "Test ☆ グルピング",
            "REPLAYGAIN_TRACK_GAIN": "-5.09 dB",
            "REPLAYGAIN_TRACK_PEAK": "0.891234"
        },
    )
    mock_run_metaflac.assert_called_once_with([
        "--export-tags-to=-",
        "audio.flac",
    ])

  @patch("media._run_metaflac")
  def test_returns_empty_dict_when_no_tags_are_present(self, mock_run_metaflac: MagicMock):
    mock_run_metaflac.return_value = subprocess.CompletedProcess(
        args=["metaflac"],
        returncode=0,
        stdout="",
    )
    tags = media.read_flac_tags("audio.flac")
    self.assertEqual(tags, {})


EYED3_OUTPUT = """
---------------------------------------------------------------------------------------------------------------------------------
Time: 01:46     MPEG1, Layer III        [ 320 kb/s @ 44100 Hz - Joint stereo ]
---------------------------------------------------------------------------------------------------------------------------------
ID3 v2.4:
title: Test Title
album: Test Album - テストアルバム
disc: 1/1
UserTextFrame: [Description: BARCODE]
123456789012
UserTextFrame: [Description: GRP1]
Test ☆ グルピング
UserTextFrame: [Description: REPLAYGAIN_TRACK_GAIN]
-11.02 dB
UserTextFrame: [Description: REPLAYGAIN_TRACK_PEAK]
1.000000
FRONT_COVER Image: [Size: 101234 bytes] [Type: image/jpeg]
Description: 

---------------------------------------------------------------------------------------------------------------------------------
"""


class ReadMp3GroupingTests(unittest.TestCase):
  @patch("media._run_eyed3")
  def test_returns_grouping_value_from_grp1_user_text_frame(self, mock_run_eyed3: MagicMock):
    mock_run_eyed3.return_value = subprocess.CompletedProcess(
        args=["eyeD3"],
        returncode=0,
        stdout=EYED3_OUTPUT,
    )
    grouping = media.read_mp3_grouping("audio.mp3")
    self.assertEqual(grouping, "Test ☆ グルピング")
    mock_run_eyed3.assert_called_once_with(["audio.mp3"])

  @patch("media._run_eyed3")
  def test_returns_empty_string_when_grp1_frame_is_missing_or_incomplete(self, mock_run_eyed3: MagicMock):
    mock_run_eyed3.return_value = subprocess.CompletedProcess(
        args=["eyeD3"],
        returncode=0,
        stdout="\n".join([
            "UserTextFrame: Description: TIT1",
            "Work Value",
            "UserTextFrame: Description: GRP1",
        ]),
    )
    grouping = media.read_mp3_grouping("audio.mp3")
    self.assertEqual(grouping, "")


if __name__ == "__main__":
  unittest.main()
