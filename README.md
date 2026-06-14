# incisor

Flash OS images to SD cards and USB drives, safely and easily.

A native desktop application built with Rust and GPUI, inspired by [balenaEtcher](https://github.com/balena-io/etcher).

## Features

- **Select source** — pick a disk image from file or paste a URL
- **Select target** — choose one or more drives from the detected list
- **Flash** — write image to block device with real-time progress (speed, ETA)
- **Verify** — SHA-256 checksum verification after write
- **Cancel** — abort in-progress flash at any time
- **Compressed images** — auto-decompresses `.gz`, `.bz2`, `.xz`, `.zip`
- **Drive safety** — warns on system drives, large drives, read-only drives

## Building

```bash
cargo build --release
```

Requires Rust and the system dependencies for GPUI (see crepuscularity docs).

### macOS

Writing to block devices requires root. Run with:

```bash
sudo cargo run --release
```

Or build and run the binary with `sudo`.

## Development

```bash
cargo run           # build and run
cargo test          # run unit tests (41)
crepus dev --bin incisor --dev  # hot-reload loop
```

## License

MPL-2.0. Derived from [balenaEtcher](https://github.com/balena-io/etcher) (Apache-2.0).
