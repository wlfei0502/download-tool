# aria2 binaries

`download-core` looks up `aria2c` in this order:

1. `DOWNLOAD_TOOL_ARIA2C` — path to an `aria2c` binary (used as given).
2. The optional override argument to `resolve_aria2c_path`.
3. `vendor/aria2/<rust-target-triple>/aria2c`, or `aria2c.exe` on Windows.

The third location is searched by walking parent directories from the process current directory and from the running executable. The directory name is the Rust target triple from the build (`rustc -vV` shows it as `host:` for a native build).

Do not commit the binaries unless the team agrees. Official builds are large. CI should download the matching archive into this tree and leave it untracked.

## Layout

```text
vendor/aria2/
  aarch64-apple-darwin/aria2c
  x86_64-apple-darwin/aria2c
  x86_64-unknown-linux-gnu/aria2c
  aarch64-unknown-linux-gnu/aria2c
  x86_64-pc-windows-msvc/aria2c.exe
```

Add other triples the same way when you build for them. The file name is `aria2c` except on Windows, where it is `aria2c.exe`. Mark Unix binaries executable (`chmod +x aria2c`).

## Official builds

Download a release archive from the aria2 project:

https://github.com/aria2/aria2/releases

Extract the `aria2c` (or `aria2c.exe`) binary into the directory for that target triple. Confirm it runs:

```text
aria2c --version
```

## Local development without a vendored binary

```text
export DOWNLOAD_TOOL_ARIA2C="$(command -v aria2c)"
```

## CI

In the job, download the official archive for the runner's target triple and unpack it to `vendor/aria2/<triple>/`. Do not commit the downloaded file.
