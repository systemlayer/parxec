import io
import sys
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import cropcalc


class ParseResolutionTests(unittest.TestCase):
  def test_parses_resolution_case_insensitively(self) -> None:
    self.assertEqual(cropcalc.parse_resolution("1920X1080"), (1920, 1080))

  def test_rejects_non_positive_dimensions(self) -> None:
    for value in ("0x1080", "1920x0", "-1x1080", "1920x-1"):
      with self.subTest(value=value):
        with self.assertRaisesRegex(ValueError, "resolution dimensions must be positive"):
          cropcalc.parse_resolution(value)

  def test_rejects_malformed_resolution(self) -> None:
    for value in ("1920", "1920x1080x1", "widthxheight"):
      with self.subTest(value=value):
        with self.assertRaises(ValueError):
          cropcalc.parse_resolution(value)


class ParseCropTests(unittest.TestCase):
  def test_parses_crop_in_top_right_bottom_left_order(self) -> None:
    self.assertEqual(cropcalc.parse_crop("1:2:3:4"), (1, 2, 3, 4))

  def test_rejects_negative_crop(self) -> None:
    with self.assertRaisesRegex(ValueError, "crop values must not be negative"):
      cropcalc.parse_crop("1:2:-3:4")

  def test_rejects_malformed_crop(self) -> None:
    for value in ("1:2:3", "1:2:3:4:5", "top:2:3:4"):
      with self.subTest(value=value):
        with self.assertRaises(ValueError):
          cropcalc.parse_crop(value)


class ResolutionCalculationTests(unittest.TestCase):
  def test_precomputed_practical_odd_and_even_resolutions(self) -> None:
    cases = (
      # input, practical (odd allowed), practical even
      ((720, 576), (720, 405), (720, 406)),
      ((853, 480), (853, 480), (852, 480)),
      ((1000, 1000), (1000, 563), (1000, 564)),
      ((1280, 720), (1280, 720), (1280, 720)),
      ((1708, 957), (1702, 957), (1700, 956)),
      ((1921, 1080), (1920, 1080), (1920, 1080)),
      ((3840, 2161), (3840, 2160), (3840, 2160)),
    )

    for input_resolution, practical, practical_even in cases:
      with self.subTest(input_resolution=input_resolution):
        self.assertEqual(cropcalc.closest_practical_16_9(*input_resolution), practical)
        self.assertEqual(
          cropcalc.closest_practical_16_9(*input_resolution, even=True),
          practical_even,
        )

  def test_precomputed_exact_odd_and_even_resolutions(self) -> None:
    cases = (
      # input, exact (odd allowed), exact even
      ((720, 576), (720, 405), (704, 396)),
      ((853, 480), (848, 477), (832, 468)),
      ((1000, 1000), (992, 558), (992, 558)),
      ((1280, 720), (1280, 720), (1280, 720)),
      ((1708, 957), (1696, 954), (1696, 954)),
      ((1921, 1080), (1920, 1080), (1920, 1080)),
      ((3840, 2161), (3840, 2160), (3840, 2160)),
    )

    for input_resolution, exact, exact_even in cases:
      with self.subTest(input_resolution=input_resolution):
        self.assertEqual(cropcalc.exact_16_9(*input_resolution), exact)
        self.assertEqual(
          cropcalc.exact_16_9(*input_resolution, even=True),
          exact_even,
        )

  def test_ceil_div_rounds_up(self) -> None:
    self.assertEqual(cropcalc.ceil_div(10, 3), 4)
    self.assertEqual(cropcalc.ceil_div(9, 3), 3)

  def test_practical_resolution_keeps_exact_16_9_input(self) -> None:
    self.assertEqual(cropcalc.closest_practical_16_9(1920, 1080), (1920, 1080))

  def test_practical_resolution_crops_wide_input(self) -> None:
    self.assertEqual(cropcalc.closest_practical_16_9(2000, 1000), (1778, 1000))

  def test_practical_resolution_crops_tall_input(self) -> None:
    self.assertEqual(cropcalc.closest_practical_16_9(1000, 1000), (1000, 563))

  def test_practical_even_resolution_has_even_dimensions(self) -> None:
    self.assertEqual(cropcalc.closest_practical_16_9(1707, 957, even=True), (1700, 956))

  def test_exact_resolution_uses_largest_fitting_factor(self) -> None:
    self.assertEqual(cropcalc.exact_16_9(1707, 957), (1696, 954))

  def test_exact_even_resolution_uses_even_factor(self) -> None:
    self.assertEqual(cropcalc.exact_16_9(1919, 1079, even=True), (1888, 1062))

  def test_exact_resolution_rejects_input_that_is_too_small(self) -> None:
    with self.assertRaisesRegex(ValueError, "resolution is too small for an exact 16:9 crop"):
      cropcalc.exact_16_9(15, 9)

    with self.assertRaisesRegex(ValueError, "resolution is too small for an exact 16:9 crop"):
      cropcalc.exact_16_9(16, 9, even=True)


class CropFilterTests(unittest.TestCase):
  def test_balanced_offset_centers_crop(self) -> None:
    self.assertEqual(cropcalc.balanced_crop_offset(12, 0, 0), 6)

  def test_balanced_offset_honors_start_and_end_minimums(self) -> None:
    self.assertEqual(cropcalc.balanced_crop_offset(12, 8, 0), 8)
    self.assertEqual(cropcalc.balanced_crop_offset(12, 0, 8), 4)

  def test_ffmpeg_filter_combines_requested_and_additional_crop(self) -> None:
    self.assertEqual(
      cropcalc.ffmpeg_crop(1708, 960, 1696, 954, 1, 0, 2, 0),
      '-vf "crop=iw-12:ih-6:6:3"',
    )


class OutputTests(unittest.TestCase):
  def test_print_table_aligns_values_and_rule(self) -> None:
    output = io.StringIO()

    with redirect_stdout(output):
      cropcalc.print_table("Title", {"A": "one", "Long": "two"})

    self.assertEqual(
      output.getvalue(),
      "Title\n-----------\nA       one\nLong    two\n",
    )

  def test_main_prints_all_resolution_and_filter_variants(self) -> None:
    output = io.StringIO()

    with patch.object(sys, "argv", ["cropcalc.py", "1708x960", "1:0:2:0"]):
      with redirect_stdout(output):
        cropcalc.main()

    result = output.getvalue()
    self.assertIn("Input resolution                             1708x960", result)
    self.assertIn("Closest practical 16:9 resolution            1702x957", result)
    self.assertIn("Closest practical even 16:9 resolution       1700x956", result)
    self.assertIn("Mathematically exact 16:9 resolution         1696x954", result)
    self.assertIn('Mathematically exact even 16:9    -vf "crop=iw-12:ih-6:6:3"', result)

  def test_main_uses_zero_crop_by_default(self) -> None:
    output = io.StringIO()

    with patch.object(sys, "argv", ["cropcalc.py", "1920x1080"]):
      with redirect_stdout(output):
        cropcalc.main()

    self.assertIn("Input resolution after crop                  1920x1080", output.getvalue())

  def test_main_rejects_invalid_argument_count(self) -> None:
    for argv in (["cropcalc.py"], ["cropcalc.py", "1920x1080", "0:0:0:0", "extra"]):
      with self.subTest(argv=argv):
        with patch.object(sys, "argv", argv):
          with self.assertRaisesRegex(ValueError, "usage:"):
            cropcalc.main()

  def test_main_rejects_crop_that_removes_entire_input(self) -> None:
    with patch.object(sys, "argv", ["cropcalc.py", "100x100", "50:0:50:0"]):
      with self.assertRaisesRegex(ValueError, "crop removes the entire input resolution"):
        cropcalc.main()


if __name__ == "__main__":
  unittest.main()
