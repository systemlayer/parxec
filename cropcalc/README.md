# cropcalc

`cropcalc` calculates crop-only FFmpeg filters that turn a video frame into a
16:9 image. It never scales or stretches the input.

For each input it reports four possible results:

- the closest practical 16:9 approximation, with the smallest additional crop;
- the same practical result with even dimensions for codec compatibility;
- a mathematically exact 16:9 resolution of the form `16*n` by `9*n`;
- the same exact result with even dimensions.

The generated filters use FFmpeg's `iw` and `ih` variables, so they can be
copied directly into a command without embedding the source dimensions.

## Usage

```text
python3 cropcalc.py WIDTHxHEIGHT [TOP:RIGHT:BOTTOM:LEFT]
```

The optional crop consists of non-negative pixel counts in
**top, right, bottom, left** order. These are minimum crop requirements, not
fixed offsets: `cropcalc` may remove more pixels to reach 16:9, and distributes
the final crop as evenly as possible while honoring every supplied minimum.

Run it from this directory, for example:

```sh
python3 cropcalc.py 1708x960 1:0:2:0
```

The output lists the candidate dimensions followed by ready-to-use filters,
such as:

```text
-vf "crop=iw-12:ih-6:6:3"
```

Copy the filter for the strategy you want into an FFmpeg command:

```sh
ffmpeg -i input.mkv -vf "crop=iw-12:ih-6:6:3" -c:a copy output.mkv
```

## Choosing a result

The **practical** result removes as few pixels as possible and can be only an
integer approximation of 16:9. For example, a frame may become `1704x958`,
whose ratio is close to, but not exactly, 16:9.

The **mathematically exact** result may crop more, but its dimensions retain an
exact 16:9 ratio when proportionally scaled. The **even** variants additionally
ensure both dimensions are divisible by two, which is required or preferred by
many video codecs.

## Testing

The test suite uses Python's built-in `unittest` framework and requires no
third-party packages. Run it from this directory:

```sh
python3 -m unittest -v test_cropcalc.py
```
