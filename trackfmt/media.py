import subprocess


def _run_metaflac(args: list[str]) -> subprocess.CompletedProcess[str]:
  return subprocess.run(
      ["metaflac", *args],
      check=False,
      stdout=subprocess.PIPE,
      stderr=subprocess.DEVNULL,
      text=True,
  )


def _run_eyed3(args: list[str]) -> subprocess.CompletedProcess[str]:
  return subprocess.run(
      ["eyeD3", "--no-color", *args],
      check=False,
      stdout=subprocess.PIPE,
      stderr=subprocess.DEVNULL,
      text=True,
  )


def read_flac_tags(file_path: str) -> dict[str, str]:
  result = _run_metaflac(["--export-tags-to=-", file_path])
  tags: dict[str, str] = {}
  for line in result.stdout.strip().splitlines():
    if "=" in line:
      key, value = line.split("=", 1)
      tags[key.strip()] = value.strip()
  return tags


def read_mp3_grouping(file_path: str) -> str:
  result = _run_eyed3([file_path])
  # Keep in mind that some applications (including Picard) set grouping with
  # the wrong identifier "TIT1" (this is because fucking Apple did it around
  # 2015). "TIT1" is used for "work" and Navidrome shows "work" (as expected),
  # the correct identifier for "grouping" is "GRP1".
  lines = result.stdout.splitlines()
  for index, line in enumerate(lines):
    is_grouping = "UserTextFrame:" in line and "Description: GRP1" in line
    if not is_grouping:
      continue
    next_line = lines[index + 1] if index < len(lines) - 1 else ""
    return next_line.strip()
  return ""
