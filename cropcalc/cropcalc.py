#!/usr/bin/env python3

# Calculate crop-only FFmpeg filters for practical and mathematically exact 16:9 resolutions.
# Example: ./cropcalc 1708x960 1:0:2:0
#
# The script never scales or stretches the image. It only crops pixels.
#
# The optional crop parameter uses top:right:bottom:left order. These values are
# treated as minimum crop requirements rather than fixed final crop positions.
#
# Two 16:9 strategies are calculated:
#
# Practical 16:9:
#   Crop the fewest pixels possible while making the remaining integer
#   dimensions as close as possible to a 16:9 aspect ratio. The resulting
#   resolution may only approximate 16:9, such as 1704x958.
#
#   Because 1704x958 is not mathematically exact 16:9, software that preserves
#   its actual aspect ratio may scale it to 1920x1079 rather than 1920x1080.
#
# Mathematically exact 16:9:
#   Require width:height to be exactly 16:9 using integer dimensions of the
#   form 16*n by 9*n. This can require cropping more pixels than the practical
#   strategy, but proportional scaling remains exactly 16:9. For example,
#   1696x954 can scale proportionally to exactly 1920x1080.
#
# Even variants use the same respective strategies while requiring both final
# dimensions to be even, which is useful for many video codecs.
#
# The final total crop is distributed as evenly as possible between opposite
# sides, while always satisfying the supplied crop values as minimums.
#
# FFmpeg output uses iw-VALUE and ih-VALUE instead of hard-coded dimensions.

import sys


def parse_resolution(value: str) -> tuple[int, int]:
  width_text, height_text = value.lower().split("x")
  width = int(width_text)
  height = int(height_text)

  if width <= 0 or height <= 0:
    raise ValueError("resolution dimensions must be positive")

  return width, height


def parse_crop(value: str) -> tuple[int, int, int, int]:
  top, right, bottom, left = map(int, value.split(":"))

  if min(top, right, bottom, left) < 0:
    raise ValueError("crop values must not be negative")

  return top, right, bottom, left


def ceil_div(value: int, divisor: int) -> int:
  return (value + divisor - 1) // divisor


def closest_practical_16_9(width: int, height: int, even: bool = False) -> tuple[int, int]:
  # Start with the resolution remaining after the requested minimum crop.
  # Only reduce the dimension that prevents the image from approaching 16:9,
  # so the practical strategy removes as few additional pixels as possible.
  target_width = width
  target_height = height

  if even:
    target_width -= target_width % 2
    target_height -= target_height % 2

  if target_width * 9 > target_height * 16:
    # Wider than 16:9: reduce width to the nearest integer approximation
    # without crossing past 16:9 and unnecessarily cropping more pixels.
    target_width = ceil_div(target_height * 16, 9)

    if even:
      target_width += target_width % 2

  elif target_width * 9 < target_height * 16:
    # Taller than 16:9: apply the equivalent strategy to the height.
    target_height = ceil_div(target_width * 9, 16)

    if even:
      target_height += target_height % 2

  return target_width, target_height


def exact_16_9(width: int, height: int, even: bool = False) -> tuple[int, int]:
  # Exact integer 16:9 dimensions must be 16*n by 9*n. Choose the largest such
  # rectangle that fits inside the resolution remaining after the minimum crop.
  factor = min(width // 16, height // 9)

  # Width is always even, but 9*n is even only when n is even.
  if even:
    factor -= factor % 2

  if factor < 1:
    raise ValueError("resolution is too small for an exact 16:9 crop")

  return factor * 16, factor * 9


def balanced_crop_offset(total_crop: int, start_min: int, end_min: int) -> int:
  # Treat the supplied crop values as minimums. Center the final total crop
  # whenever possible, shifting it only when necessary to preserve a minimum.
  min_start = start_min
  max_start = total_crop - end_min
  balanced_start = total_crop // 2

  return min(max(balanced_start, min_start), max_start)


def ffmpeg_crop(width: int, height: int, target_width: int, target_height: int, top: int, right: int, bottom: int, left: int) -> str:
  # Express the total crop relative to the original input. The requested crop
  # and any additional aspect-ratio crop are combined into one FFmpeg filter.
  width_crop = width - target_width
  height_crop = height - target_height

  # Balance the complete final crop between opposite sides while still covering
  # every side by at least the amount requested in the crop parameter.
  x = balanced_crop_offset(width_crop, left, right)
  y = balanced_crop_offset(height_crop, top, bottom)

  return f'-vf "crop=iw-{width_crop}:ih-{height_crop}:{x}:{y}"'


def print_table(title: str, values: dict[str, str]) -> None:
  label_width = max(map(len, values))
  table_width = max(
    len(title),
    max(label_width + 2 + len(value) for value in values.values()),
  )

  print(title)
  print("-" * table_width)

  for label, value in values.items():
    print(f"{label:<{label_width}}  {value}")


def main() -> None:
  args = sys.argv[1:]

  if not 1 <= len(args) <= 2:
    raise ValueError(f"usage: {sys.argv[0]} WIDTHxHEIGHT [TOP:RIGHT:BOTTOM:LEFT]")

  width, height = parse_resolution(args[0])

  crop_value = args[1] if len(args) == 2 else "0:0:0:0"
  top, right, bottom, left = parse_crop(crop_value)

  # Apply the supplied crop as a set of minimum requirements before calculating
  # either practical or exact 16:9 target resolutions.
  cropped_width = width - left - right
  cropped_height = height - top - bottom

  if cropped_width <= 0 or cropped_height <= 0:
    raise ValueError("crop removes the entire input resolution")

  practical_width, practical_height = closest_practical_16_9(cropped_width, cropped_height)
  practical_even_width, practical_even_height = closest_practical_16_9(cropped_width, cropped_height, even=True)
  exact_width, exact_height = exact_16_9(cropped_width, cropped_height)
  exact_even_width, exact_even_height = exact_16_9(cropped_width, cropped_height, even=True)

  practical_crop = ffmpeg_crop(width, height, practical_width, practical_height, top, right, bottom, left)
  practical_even_crop = ffmpeg_crop(width, height, practical_even_width, practical_even_height, top, right, bottom, left)
  exact_crop = ffmpeg_crop(width, height, exact_width, exact_height, top, right, bottom, left)
  exact_even_crop = ffmpeg_crop(width, height, exact_even_width, exact_even_height, top, right, bottom, left)

  resolutions = {
    "Input resolution:": f"{width}x{height}",
    "Input resolution after crop:": f"{cropped_width}x{cropped_height}",
    "Closest practical 16:9 resolution:": f"{practical_width}x{practical_height}",
    "Closest practical even 16:9 resolution:": f"{practical_even_width}x{practical_even_height}",
    "Mathematically exact 16:9 resolution:": f"{exact_width}x{exact_height}",
    "Mathematically exact even 16:9 resolution:": f"{exact_even_width}x{exact_even_height}",
  }

  crop_filters = {
    "Closest practical 16:9:": practical_crop,
    "Closest practical even 16:9:": practical_even_crop,
    "Mathematically exact 16:9:": exact_crop,
    "Mathematically exact even 16:9:": exact_even_crop,
  }

  print_table("Dimensions", resolutions)
  print()
  print_table("FFmpeg crop filters", crop_filters)


if __name__ == "__main__":
  main()
