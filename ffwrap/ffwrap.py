#!/usr/bin/env python3

# FFmpeg wrapper that extracts a contiguous chapter range while forwarding all
# non-wrapper arguments unchanged to ffmpeg. Input and output paths are supplied
# normally as part of the ffmpeg arguments.
#
# Examples:
#   ffwrap -x-chapter-range 3:7 -i input.mkv -map 0 -c copy output.mkv
#   ffwrap -x-dry-run -x-chapter-range 3:7 -i input.mkv -map 0 -c copy output.mkv
#
# The first example extracts chapters 3 through 6 into output.mkv.
# The second prints the resulting ffmpeg command without executing it.

import json
import shlex
import subprocess
import sys
from collections.abc import Callable
from typing import Any


def parse_chapter_range(value: str) -> tuple[int, int]:
  start, end = value.split(":", 1)
  return int(start), int(end)


XOptionParser = Callable[[list[str]], Any]

# Wrapper-specific -x-* options live here so additional commands can be added
# without changing the generic argument-processing loop.
X_OPTION_SPECS: dict[str, tuple[int, XOptionParser]] = {
  "-x-chapter-range": (1, lambda values: parse_chapter_range(values[0])),
  "-x-dry-run": (0, lambda values: True),
}


def parse_args(args: list[str]) -> tuple[dict[str, Any], list[str]]:
  x_options: dict[str, Any] = {}
  ffmpeg_args: list[str] = []

  i = 0
  while i < len(args):
    arg = args[i]

    if not arg.startswith("-x-"):
      ffmpeg_args.append(arg)
      i += 1
      continue

    spec = X_OPTION_SPECS.get(arg)
    if spec is None:
      raise ValueError(f"unknown wrapper option: {arg}")

    if arg in x_options:
      raise ValueError(f"{arg} specified more than once")

    count, parser = spec
    values = args[i + 1:i + 1 + count]

    if len(values) != count:
      raise ValueError(f"{arg} requires {count} argument(s)")

    x_options[arg] = parser(values)
    i += 1 + count

  return x_options, ffmpeg_args


def main() -> None:
  x_options, ffmpeg_args = parse_args(sys.argv[1:])

  chapter_range = x_options.get("-x-chapter-range")
  if chapter_range is None:
    raise ValueError("-x-chapter-range START:END is required")

  input_option_index = ffmpeg_args.index("-i")
  input_path = ffmpeg_args[input_option_index + 1]

  start_chapter, end_chapter = chapter_range

  probe_command: list[str] = [
    "ffprobe",
    "-v", "error",
    "-show_entries", "chapter=start_time,end_time",
    "-of", "json",
    input_path,
  ]

  print(shlex.join(probe_command), file=sys.stderr, flush=True)

  probe = subprocess.run(
    probe_command,
    check=True,
    capture_output=True,
    text=True,
  )

  chapters = json.loads(probe.stdout)["chapters"]

  if not 1 <= start_chapter < end_chapter <= len(chapters) + 1:
    raise ValueError(
      f"chapter range must satisfy "
      f"1 <= start < end <= {len(chapters) + 1}"
    )

  start = chapters[start_chapter - 1]["start_time"]

  if end_chapter <= len(chapters):
    end = chapters[end_chapter - 1]["start_time"]
  else:
    end = chapters[-1]["end_time"]

  command: list[str] = [
    "ffmpeg",
    "-loglevel", "warning",
    "-stats",
    *ffmpeg_args[:input_option_index],
    "-ss", start,
    "-to", end,
    *ffmpeg_args[input_option_index:],
  ]

  # Print diagnostics to stderr so ffmpeg stdout remains clean, including when
  # stdout is used for media output with "-".
  print(shlex.join(command), file=sys.stderr, flush=True)

  if x_options.get("-x-dry-run", False):
    return

  # stdin/stdout/stderr are intentionally not specified, so ffmpeg inherits
  # them directly from this process (normally the current terminal).
  subprocess.run(command, check=True)


if __name__ == "__main__":
  main()
