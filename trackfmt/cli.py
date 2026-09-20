#!/usr/bin/env python3

import argparse
import os
import subprocess
import sys
from pathlib import Path, PosixPath
from media import read_flac_tags, read_mp3_grouping


def existing_dir(path_str: str) -> Path:
  path = Path(path_str)
  if not path.is_dir():
    raise argparse.ArgumentTypeError(f"not a directory: {path}")
  return path


def build_parser() -> argparse.ArgumentParser:
  parser = argparse.ArgumentParser(prog="trackfmt")
  parser.add_argument(
      "--non-interactive",
      action="store_true",
      help="Run without waiting for confirmation.",
  )
  subparsers = parser.add_subparsers(dest="command", required=True)

  commands = {
      "grouping": "Set the grouping tag on audio files.",
      "gain": "Add ReplayGain metadata to audio files.",
      "dedup": "Scan for duplicate audio files.",
  }

  for name, help_text in commands.items():
    subparser = subparsers.add_parser(
        name, help=help_text, description=help_text)
    subparser.add_argument(
        "directory",
        nargs="?",
        help="Path to a directory containing audio files. Optional when MEDIA_PATH is set.",
    )
    if name == "grouping":
      subparser.add_argument(
          "--dry-run",
          action="store_true",
          help="Show what would be done without making changes.",
      )
    if name == "gain":
      subparser.add_argument(
          "--force",
          action="store_true",
          help="Recalculate ReplayGain metadata even when tags already exist.",
      )
    if name == "dedup":
      subparser.add_argument(
          "--length",
          type=int,
          default=120,
          help="Number of seconds to analyze (default: 120).",
      )

  return parser


def resolve_directory(
    parser: argparse.ArgumentParser,
    args: argparse.Namespace
) -> Path:
  media_path = os.environ.get("MEDIA_PATH")
  if media_path:
    return existing_dir(media_path)
  if args.directory is None:
    parser.error("the following arguments are required: directory")
  return existing_dir(args.directory)


def _press_return_to_continue() -> None:
  if not sys.stdin.isatty():
    sys.exit("ERROR: Interactive confirmation requires a TTY.")
  input("Press Return to continue...")


def handle_grouping(args: argparse.Namespace) -> None:
  base_dir: PosixPath = args.directory.resolve()
  skipped_names = {"cover.jpg", "cover.png", ".ndignore"}

  print(f"Scanning directory: {args.directory}")
  entries: list[Path] = sorted(args.directory.rglob("*"))

  for file_path in entries:
    if not file_path.is_file():
      continue

    if file_path.name in skipped_names:
      continue

    file_abs = file_path.resolve()
    try:
      relative = file_abs.relative_to(base_dir).as_posix()
    except ValueError:
      sys.exit(f"ERROR: File not under BASE_DIR (unexpected): {file_path}")

    # Parse expected 4 components.
    # e.g., "game/Super Tux Kart/SuperTux SOUNDTRACK/track1.mp3".
    parts = relative.split("/", 3)
    if len(parts) < 4:
      sys.exit(f"ERROR: Unexpected directory structure: {file_path}")

    # Grouping can be the complete relative path, or could be stylized.
    # Using the relative path is longer and difficult to read
    # e.g., "category/collection/album".
    _category, collection, _album, audio_file = parts
    grouping = collection
    extension = Path(audio_file).suffix.lower()

    if extension == ".flac":
      current = read_flac_tags(str(file_path)).get("GROUPING")
      if current == grouping:
        if args.dry_run:
          print(f"Skipping (already tagged): {file_path}")
        continue

      print(f"Tagging FLAC: {file_path} (Grouping={grouping})")
      if not args.dry_run:
        subprocess.run(
            [
                "metaflac",
                "--remove-tag=GROUPING",
                f"--set-tag=GROUPING={grouping}",
                str(file_path),
            ],
            check=True,
        )
      continue

    if extension == ".mp3":
      current = read_mp3_grouping(str(file_path))
      if current == grouping:
        if args.dry_run:
          print(f"Skipping (already tagged): {file_path}")
        continue

      print(f"Tagging MP3: {file_path} (Grouping={grouping})")
      if not args.dry_run:
        subprocess.run(
            [
                "eyeD3",
                "--user-text-frame",
                f"GRP1:{grouping}",
                str(file_path),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
      continue

    print(f"WARNING: Non-audio file found: {file_path}")


def handle_gain(args: argparse.Namespace) -> None:
  skip_existing = [] if args.force else ["--skip-existing"]
  subprocess.run(
      [
          "rsgain",
          "easy",
          "-p",
          "no_album",
          "-m",
          "MAX",
          *skip_existing,
          str(args.directory),
      ],
      check=True,
  )


def handle_dedup(args: argparse.Namespace) -> None:
  print(f"Scanning directory: {args.directory}")
  audio_files = sorted(
      file_path.relative_to(args.directory).as_posix()
      for file_path in args.directory.rglob("*")
      if file_path.is_file() and file_path.suffix.lower() in {".mp3", ".flac"}
  )
  subprocess.run(
      [
          "audiomatch",
          "--length",
          str(args.length),
          *audio_files,
      ],
      check=True,
      cwd=args.directory,
  )


def main() -> None:
  parser = build_parser()
  args = parser.parse_args()
  args.directory = resolve_directory(parser, args)
  handlers = {
      "grouping": handle_grouping,
      "gain": handle_gain,
      "dedup": handle_dedup,
  }
  if not args.non_interactive:
    _press_return_to_continue()
  handlers[args.command](args)


if __name__ == "__main__":
  try:
    main()
  except KeyboardInterrupt:
    print("\nInterrupted.", file=sys.stderr)
    raise SystemExit(130)
