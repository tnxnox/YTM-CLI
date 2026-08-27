# 🎧 YTM-CLI: YouTube Music CLI Player

[![CI](https://github.com/tnxnox/YTM-CLI/actions/workflows/ci.yml/badge.svg)](https://github.com/tnxnox/YTM-CLI/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024-black?logo=rust)](https://www.rust-lang.org/)
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
- **🎤 Real-Time Synced Lyrics:** Automatic multi-provider timestamped lyrics engine (LRCLib & cache) synced with playback, highlighting the current line in real time.
- **🐧 Linux MPRIS & Media Key Support:** Full D-Bus `org.mpris.MediaPlayer2` integration allowing hardware media keys (Play/Pause, Next, Previous, Seek) and desktop widgets (`playerctl`, Waybar, GNOME/KDE, OS media controls) to control playback.
- **❤️ Favorites & Liked Songs:** Save and manage favorite tracks with one keypress (`[F]`), browse them in an interactive table, or play them with shuffle.
- **📁 Custom Local Playlists:** Create and manage offline playlists stored in SQLite without needing a YouTube account. Add any playing song directly with `[A]`.
- **📜 Interactive Live Queue Viewer:** Press `[Tab]` during playback to view upcoming tracks, jump directly to any song, or remove items from the queue.
- **🔁 Advanced Playback Modes:** Repeat Single Track (`🔂`), Repeat Entire Queue (`🔁`), Mute toggle (`[M]`), and on-the-fly queue shuffling (`[S]`).
- **💿 Album Search & Playback:** Search for albums/EPs directly, select an album from the interactive menu, and play all its tracks in sequence.
- **📻 Smart Autoplay (Infinite Queue):** Automatically creates a radio queue based on your selected track and dynamically fetches continuations in the background as you play.
- **🤖 Discord Selfbot & RPC Mode:** Remote control your Discord `Jockie Music` bot directly from your terminal and showcase active tracks on Discord Rich Presence.
- **⚡ Background Prefetching & Caching:** Automatically pre-downloads the next track in the queue in the background for zero-latency, gapless transitions.
- **🔑 Account Integration:** Seamless cookie extraction from local browsers (Firefox, Chrome, Brave, Chromium, Edge, Opera, Vivaldi) to load and play your private library playlists.

---

## ⌨️ Controls

During playback, you can control the music in real time:

| Key | Action |
| --- | --- |
| `[Space]` | Play / Pause |
| `[←]` / `[→]` | Seek Backward / Forward (5s) |
| `[↑]` / `[↓]` | Increase / Decrease Volume |
| `[M]` | Mute / Unmute Volume |
| `[R]` | Cycle Repeat Mode (Off ➔ 🔂 Track ➔ 🔁 Queue) |
| `[S]` | Shuffle Upcoming Tracks in Queue |
| `[Tab]` | Open Interactive Live Queue Viewer & Track Jump |
| `[F]` | Toggle Favorite / Liked Song (❤️) |
| `[A]` | Add Current Track to Local Playlist |
| `[L]` | Toggle Synced Lyrics Display |
| `[N]` | Skip to Next Track |
| `[P]` | Go back to Previous Track |
| `[Q]` / `[Esc]` | Stop Playback & Go Back |

---

## 🚀 Installation & Requirements

### Dependencies
You must have the following tools installed and available in your `PATH`:
1. Python 3 (required for `yt-dlp` virtual environment)
2. `ffmpeg` (required for audio extraction)

*Note: You do not need to install `yt-dlp` manually. The application automatically creates a Python virtual environment and installs/updates `yt-dlp` locally on the first run (cross-platform compatible).*

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
- **Play an album entirely by name:**
  ```bash
  ytm-cli album "Nevermind"
  ```
- **Play or shuffle your favorite / liked songs:**
  ```bash
  ytm-cli favorites
  ytm-cli favorites --shuffle
  ```
- **Play or manage custom local playlists:**
  ```bash
  ytm-cli local-playlist "Workout Mix"
  ytm-cli local-playlist "Workout Mix" --shuffle
  ```
- **Play a public/private playlist (with optional shuffle):**
  ```bash
  ytm-cli playlist "https://music.youtube.com/playlist?list=..." --shuffle
  ```
- **Search and print a table of tracks:**
  ```bash
  ytm-cli search "Rim'K Air Max"
  ```
- **Search and print a table of matching albums:**
  ```bash
  ytm-cli search-album "Nevermind"
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
