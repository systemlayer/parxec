# ffwrap

`ffwrap` is a small FFmpeg wrapper for extracting a contiguous range of
chapters. It uses `ffprobe` to find the chapter timestamps, inserts the
corresponding `-ss` and `-to` options into the command, and forwards all other
arguments to FFmpeg unchanged.

## Requirements

- Python 3.9 or later
- `ffmpeg` and `ffprobe` available on `PATH`
- an input containing chapter metadata

## Usage

```text
python3 ffwrap.py -x-chapter-range START:END [FFMPEG_ARGUMENTS...]
```

`START` is the first chapter to include. `END` is exclusive, so `3:7` extracts
chapters 3, 4, 5, and 6. Chapter numbers are one-based. To extract through the
last chapter, use one more than the number of chapters as `END`.

Supply the input and output exactly as normal FFmpeg arguments:

```sh
python3 ffwrap.py \
  -x-chapter-range 3:7 \
  -i input.mkv \
  -map 0 -c copy \
  output.mkv
```

The wrapper prints both the `ffprobe` command and the resulting FFmpeg command
to standard error. FFmpeg then inherits the terminal's standard input, output,
and error streams normally.

## Wrapper options

- `-x-chapter-range START:END` — required; selects the half-open chapter range
  from `START` through `END - 1`.
- `-x-dry-run` — prints the command without executing FFmpeg. The input is still
  inspected with `ffprobe` so the timestamps and range can be validated.

For example:

```sh
python3 ffwrap.py \
  -x-dry-run \
  -x-chapter-range 3:7 \
  -i input.mkv \
  -map 0 -c copy \
  output.mkv
```

All wrapper-specific options begin with `-x-`. Any unknown `-x-` option is
rejected; all other arguments are passed through to FFmpeg. Each wrapper option
may be specified only once, and an `-i INPUT` argument is required.

## Notes

The generated seek options are placed before `-i`, making this an input seek.
With stream copying (`-c copy`), the exact cut point can therefore depend on
available keyframes and the source format. Re-encode when frame-accurate cuts
are required.
