# Artisan

Flash OS images to SD cards and USB drives, safely and easily.

A native desktop application built with Rust and GPUI, inspired by [balenaEtcher](https://github.com/balena-io/etcher).

## Acknowledgments

Artisan is heavily inspired by and acknowledges the excellent work done by the
[balenaEtcher](https://github.com/balena-io/etcher) team. The application
architecture, UX patterns, and drive compatibility logic are derived from their
open-source project (Apache 2.0).

## Building

```bash
cargo build --release
```

Requires Rust and the system dependencies for GPUI (see crepuscularity docs).
