# trackfmt

`trackfmt` is a small command-line tool for organizing an audio library. It can
set MP3/FLAC grouping tags, add ReplayGain metadata, and find duplicate tracks.

## Required tools

The tools needed for the commands you use must be available on `PATH`:

- `metaflac`: Reads and updates grouping tags in FLAC files for `grouping`.
- `eyeD3`: Reads and updates grouping tags in MP3 files for `grouping`.
- `rsgain`: Calculates and writes ReplayGain metadata for `gain`.
- `audiomatch`: Compares audio files to find duplicates for `dedup`.

## Development

All required tools except `audiomatch` can be installed through the `apt` or
`apk` package manager, depending on your Linux distribution.

Install `audiomatch` with `pip`. Version 0.1.8 is the latest release as of
July 2026:

```sh
python3 -m pip install --only-binary=:all: audiomatch==0.1.8
```

## Usage

```sh
python3 cli.py [--non-interactive] {grouping,gain,dedup} DIRECTORY
```

Run `python3 cli.py COMMAND --help` for command-specific options.
