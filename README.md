# Openbar5000

Openbar5000 is an open-source, cross-platform (Windows, macOS, Linux) alternative to foobar2000, built in Rust with a [Slint](https://slint.dev) UI. It aims to look and feel as close to foobar2000 as practical while using a plugin architecture of its own (not binary-compatible with real foobar2000 components).

## Features

- Cross-platform audio playback (FLAC, MP3, WAV, Ogg Vorbis, AAC, ALAC, and more via [Symphonia](https://github.com/pdeljanov/Symphonia))
- Album-art, grouped playlist view with search
- Real waveform seekbar (decoded peak data, not a mockup)
- Live spectrum analyzer (real FFT, with channel/amplitude/FFT-size/peak-gravity settings)
- Dark/Light/One Dark/Dracula themes, plus fully custom user-defined themes
- Discord Rich Presence ("Listening to \<track\>")
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

## Discord Rich Presence

Openbar5000 can show the track you're currently listening to on your Discord
profile ("Listening to \<track\>"). It's off by default, in **Preferences →
Discord Rich Presence**.

### Adding it to Discord

Rich Presence is tied to a **Discord Application** — an identity you
register once in Discord's own developer portal, which supplies the
numeric **Client ID** Openbar5000 needs and, optionally, the icon/art shown
on your profile. Steps:

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications) and log in with your Discord account.
2. Click **New Application**, give it a name (e.g. "Openbar5000"), and create it.
3. On the **General Information** page, copy the **Application ID**.
4. In Openbar5000, open **Preferences → Discord Rich Presence**, paste that ID into **Client ID**, and check **Enable Discord Rich Presence**. The status line below flips to "Connected" once it finds Discord running.
5. *(Optional)* Under **Rich Presence → Art Assets** in that application, upload images named exactly `logo`, `play`, and `pause` (square, ideally 1024×1024 for `logo`, at least 32×32 for the others) to show real artwork and a play/pause badge instead of plain text.

`DEFAULT_CLIENT_ID` in `crates/app/src/discord_presence.rs` is a project-wide
fallback: if it's set to a real Application ID (registered once by the
project, rather than by each person running the app), Openbar5000 auto-fills
it into the Client ID field the moment Rich Presence is first enabled with
that field still empty, so most people never have to do steps 1-3
themselves — it's still a plain, editable field, so pointing it at a
personal application instead just means typing over it. It ships empty,
since a working Client ID can't be generated — it's inherently tied to
whoever registers it in the portal above.

This all works by speaking Discord's own local IPC protocol directly — no
Discord-side approval needed.

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
