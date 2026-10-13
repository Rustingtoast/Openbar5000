# Openbar5000

Openbar5000 is an open-source, cross-platform (Windows, macOS, Linux) alternative to foobar2000, built in Rust with a [Slint](https://slint.dev) UI. It aims to look and feel as close to foobar2000 as practical while using a plugin architecture of its own (not binary-compatible with real foobar2000 components).

## Features

- Cross-platform audio playback (FLAC, MP3, WAV, Ogg Vorbis, AAC, ALAC, and more via [Symphonia](https://github.com/pdeljanov/Symphonia))
- Album-art, grouped playlist view with search
- Real waveform seekbar (decoded peak data, not a mockup)
- Live spectrum analyzer (real FFT, with channel/amplitude/FFT-size/peak-gravity settings)
- Dark/Light/One Dark/Dracula themes, plus fully custom user-defined themes
- A native Rust plugin API (`plugin-api` + `plugin-host`), proven end-to-end with a sample DSP plugin

## Getting Started

### Prerequisites

- **Rust** (stable toolchain) via [rustup](https://rustup.rs)
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)
- **Linux**: ALSA and windowing dev headers, e.g. on Debian/Ubuntu:
  ```bash
  sudo apt-get install -y build-essential pkg-config libasound2-dev \
      libxkbcommon-dev libxcb1-dev libx11-dev libxcursor-dev \
      libxrandr-dev libxi-dev libgl1-mesa-dev libfontconfig1-dev
  ```
- **Windows**: the MSVC build tools (installed automatically by `rustup` on Windows, or via [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/))

### Clone and build

```bash
git clone https://github.com/Rustingtoast/Openbar5000.git
cd Openbar5000
cargo build --workspace
```

### Run it

```bash
cargo run -p app
```

### Run the tests

```bash
cargo test --workspace
```

### Build a release binary

For a quick local release build:

```bash
cargo build --release -p app
```

For a packaged build placed under `dist/<platform>/`, use the scripts in [`compile/`](compile/) — see [`compile/README.md`](compile/README.md) for details and platform notes. On Debian/Ubuntu, the Linux script also produces an installable `.deb` package; on macOS, a proper `.app` bundle; on Windows, an MSI installer.

## Project layout

```
crates/
  plugin-api/    # native Rust plugin ABI (decoder/DSP/output service traits)
  plugin-host/   # dynamic plugin loading and discovery
  engine/        # playback engine: decode -> DSP -> output, playlist, waveform/spectrum analysis
  ui/            # Slint UI (.slint files) and Rust glue
  app/           # the `openbar5000` binary: wires the engine to the UI
plugins/
  gain-dsp/      # proof-of-concept Openbar5000 plugin
```

## Acknowledgments

Openbar5000's UI is built with [Slint](https://slint.dev) — a huge thanks
to the Slint team for a great Rust-native GUI toolkit, used here under its
Royalty-free license.

## License

Apache-2.0 — see [LICENSE](LICENSE).
