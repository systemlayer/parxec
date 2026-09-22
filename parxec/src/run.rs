use anyhow::Context;
use std::ffi::{OsStr, OsString};

/// Replaces supported placeholders in one UTF-8 command argument.
fn substitute(
  template: &OsStr,
  file: &OsStr,
  input: &OsStr,
  output: &OsStr,
) -> anyhow::Result<OsString> {
  // These values come from CLI arguments and filesystem paths, so invalid UTF-8 is a
  // recoverable input error rather than an invariant that warrants a panic.
  let template = template
    .to_str()
    .context("command argument is not valid UTF-8")?;
  let file = file.to_str().context("input filename is not valid UTF-8")?;
  let input = input
    .to_str()
    .context("input directory is not valid UTF-8")?;
  let output = output
    .to_str()
    .context("output directory is not valid UTF-8")?;
  Ok(
    template
      .replace("{file_name}", file)
      .replace("{input_dir}", input)
      .replace("{output_dir}", output)
      .into(),
  )
}
