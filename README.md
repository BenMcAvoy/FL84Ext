# FL84Ext

FL84Ext is a Rust reverse-engineering and overlay experiment for the Windows game client `SolarlandClient-Win64-Shipping.exe`. It uses a local driver interface, the `dumpspace-api` crate and an `egui` overlay to explore process and game-state data.

## Build

The project targets Windows. With a Rust toolchain installed, build the crate from the repository root:

```powershell
cargo build
```

## Status

This is an unfinished, game-build-specific research project. The process interface and data layouts depend on external driver and game versions; it is not a general-purpose overlay or released application.
