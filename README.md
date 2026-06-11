# 🎧 YTM-CLI: YouTube Music CLI Player

[![Rust](https://img.shields.io/badge/Rust-2021-black?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-Support-FF5E5B?logo=kofi)](https://ko-fi.com/thenoix)

A lightweight, high-performance, and feature-rich YouTube Music client for the terminal, written in Rust. It features a modern deep blue / purple command-line interface, a real-time DSP equalizer, and an infinite autoplay queue.

---

## ✨ Features

- **🎮 CLI-First Interactive TUI:** Clean, styled menus, search result tables, and playback interfaces powered by `dialoguer` and `comfy-table`.
- **📊 Real-Time Visual Equalizer:** A software-based digital signal processing (DSP) crossover network inside the audio stream:
  - Splits audio into 8 frequency bands (Sub-bass, Bass, Low Mids, Mids, Upper Mids, Presence, Treble, Brilliance).
  - Smooth envelope tracking (fast attack, slow decay).
  - Seamless RGB horizontal color gradients (Slate Blue ➔ Purple ➔ Orchid).
  - Non-linear square root amplitude compression to prevent height pegging/clipping.
  - Rendered at ~25 FPS across 5 terminal rows using Unicode block elements (` ▂▃▄▅▆▇█`).
- **📻 Smart Autoplay (Infinite Queue):** Automatically creates a radio queue based on your selected track and dynamically fetches continuations in the background as you play.
- **⚡ Background Prefetching:** Automatically pre-downloads the next track in the queue in the background for zero-latency, gapless transitions.
- **💾 Local Caching:** Audio is cached locally as `.flac` files (avoiding seeking distortion and decoder panics) and tracked in a local SQLite database.
- **🔑 Account Integration:** Seamless cookie extraction from local browsers (Firefox, Chrome, Brave, Chromium, Edge, Opera, Vivaldi) to load and play your private library playlists.
- **🚀 Performance-Optimized:**
  - Zero-allocation lazy text styling for the terminal progress bar redraw loop.
  - Thread-safe shared database connection (`Arc<Mutex<Connection>>`).
  - High-performance typed JSON deserialization bypassing heavy generic JSON mappings.

---

## ⌨️ Controls

During playback, you can control the music in real time:

| Key | Action |
| --- | --- |
| `[Space]` | Play / Pause |
| `[←]` / `[→]` | Seek Backward / Forward (10s) |
| `[↑]` / `[↓]` | Increase / Decrease Volume |
| `[N]` | Skip to Next Track |
| `[P]` | Go back to Previous Track |
| `[Q]` / `[Esc]` | Stop Playback & Go Back |

---

## 🚀 Installation & Requirements

### Dependencies
You must have the following tools installed and available in your `PATH`:
1. [yt-dlp](https://github.com/yt-dlp/yt-dlp) (used for audio extraction)
2. `ffmpeg` (required by `yt-dlp` for FLAC extraction)

### Build
Clone the repository and build the binary:
```bash
git clone https://github.com/tnxnox/YTM-CLI.git
cd YTM-CLI
cargo build --release
```
The compiled binary will be available at `./target/release/ytm-cli`.

---

## 📖 Usage

Run the player interactively to launch the terminal menu:
```bash
./target/release/ytm-cli
```

### Command Line Subcommands
You can also run one-off commands directly:

- **Play a search query immediately (triggers Autoplay):**
  ```bash
  ytm-cli play "Rim'K Air Max Ninho"
  ```
- **Search and print a table of tracks:**
  ```bash
  ytm-cli search "Rim'K Air Max"
  ```
- **Play a public/private playlist (with optional shuffle):**
  ```bash
  ytm-cli playlist "https://music.youtube.com/playlist?list=..." --shuffle
  ```
- **Manage downloaded cache:**
  ```bash
  ytm-cli cache list
  ytm-cli cache clear
  ```
- **Authenticate with your YouTube Music account:**
  ```bash
  ytm-cli login --browser firefox
  ```

---

## 🤝 Support

If you like the app, you can thank me here:

[![Ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/thenoix)

## 📄 License

MIT — see [LICENSE](LICENSE) for the full text.
