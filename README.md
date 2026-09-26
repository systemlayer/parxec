<div align="center">
  <h1>parxec</h1>
  <p>Process file collections in parallel without repeating expensive work for duplicate inputs.</p>

  [![Build](https://github.com/systemlayer/parxec/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/systemlayer/parxec/actions/workflows/build.yml)
  [![Latest release](https://img.shields.io/github/v/release/systemlayer/parxec)](https://github.com/systemlayer/parxec/releases/latest)
  [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://opensource.org/license/mit)
</div>

Parxec finds duplicate files, processes each unique file once, and creates the expected output filenames for the duplicates. It distributes the work across concurrent copies of a command, making it useful for image upscaling, conversion, inference, and other expensive batch operations.

## Installation

Download the latest Linux archive from [GitHub Releases](https://github.com/systemlayer/parxec/releases/latest), then install it:

```bash
tar -xzf parxec-<version>.tgz
install -m 0755 parxec ~/.local/bin/parxec
```

Make sure `~/.local/bin` is on your `PATH`.

To build from source, install a current Rust toolchain and run:

```bash
git clone https://github.com/systemlayer/parxec.git
cd parxec
cargo build --release --locked
install -m 0755 target/release/parxec ~/.local/bin/parxec
```

## Quick start

Create an empty output directory, then run one command per batch of unique inputs:

```bash
mkdir upscaled
parxec run frames/ --output-dir upscaled/ --jobs 6 --dry-run -- \
  realesrgan-ncnn-vulkan -i '{input_dir}' -o '{output_dir}' -s 3 -f png
```

Review the JSON execution plan, then remove `--dry-run` to process the files. Parxec replaces `{input_dir}` with a temporary directory containing a batch of inputs and `{output_dir}` with the requested output directory.

The `--` separator ends Parxec's options. Everything after it is the program and its arguments.

## Reusing hashes

`run` hashes inputs automatically. For repeated analysis or processing, hashes can instead be saved and reused:

```bash
parxec hash frames/ --hash-output frame-hashes.json
parxec analyze --hash-input frame-hashes.json --jobs 6 --file-ms 250
parxec run frames/ --output-dir upscaled/ --hash-input frame-hashes.json --jobs 6 -- \
  realesrgan-ncnn-vulkan -i '{input_dir}' -o '{output_dir}' -s 3 -f png
```

The default `downsampled` algorithm detects images with matching resized pixels. Use `--hash-algorithm sha256` for non-image files or exact-content matching:

```bash
parxec hash inputs/ --hash-output hashes.json --hash-algorithm sha256
```

## Requirements and behavior

- Only regular files directly inside the input directory are processed; subdirectories are not searched. Symbolic links are not supported.
- The output directory must already exist and be empty.
- The input and output filesystems must support hard links.
- The external program must preserve input filenames in the output directory.
- Commands run directly, without a shell. To use pipes, redirects, wildcards, or environment expansion, invoke a shell explicitly.
- A reused hash file must contain every file currently in the input directory. Entries for files no longer present are ignored.
- Successful command output is hidden. If a command fails, Parxec stops the other commands and prints up to the last 50 lines of output.
- Ctrl-C stops active commands, removes their temporary directories, and exits with status 130.

Run `parxec --help` or `parxec help <COMMAND>` for the complete command reference.

## Development

### Testing

Create 30 small files for a local test run:

```bash
mkdir -p input output
for i in {1..30}; do printf '%s\n' "$i" > "input/$i"; done
```

Run Parxec with two workers and a shell script that waits before copying each file:

```bash
cargo run -- run ./input -o ./output --hash-algorithm sha256 --jobs 2 -- \
  bash -c 'for file in "$1"/*; do sleep 2; cp -- "$file" "$2/"; done' \
  bash '{input_dir}' '{output_dir}'
```

### Dependency maintenance

Check dependencies for known security vulnerabilities:

```bash
cargo audit
```

Update compatible dependency versions in `Cargo.lock`:

```bash
cargo update
```

Preview available dependency upgrades for `Cargo.toml` (requires `cargo-edit`):

```bash
cargo upgrade --dry-run
```

### Internal branch-pushing workflow

Prune stale remote-tracking branches before pushing. This avoids name collisions when, for example, a deleted `dev` branch would prevent creating `dev/new-feature`:

```bash
git remote prune origin
```

Push the current `HEAD` to a different remote branch without changing the current branch's upstream:

```bash
git push origin HEAD:dev/new-feature
```

## License

[MIT](https://opensource.org/license/mit)
