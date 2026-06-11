pub mod config;
pub mod db;
pub mod network;
pub mod audio;
pub mod theme;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::event::{self, KeyCode, Event};
use rustypipe::model::paginator::ContinuationEndpoint;
use owo_colors::OwoColorize;

use crate::config::Config;
use crate::db::Db;
use crate::network::{NetworkClient, TrackInfo};
use crate::audio::AudioPlayer;

#[derive(Parser)]
#[command(name = "ytm-cli", version = "0.1.0", about = "YouTube Music CLI client")]
struct Cli {
    /// Enable verbose debug logging to terminal
    #[arg(short, long, global = true)]
    debug: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Play a track matching the query
    Play {
        /// The search query
        query: String,
    },
    /// Search for tracks matching the query
    Search {
        /// The search query
        query: String,
    },
    /// Cache management commands
    Cache {
        #[command(subcommand)]
        action: CacheCommands,
    },
    /// View playback history
    History {
        /// Number of history entries to show
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Log in to YouTube Music to access library
    Login {
        /// The browser to extract cookies from (e.g. firefox, chrome, chromium, brave, edge)
        #[arg(short, long, default_value = "firefox")]
        browser: String,
    },
    /// Log out from YouTube Music
    Logout,
    /// Play a playlist by URL or ID
    Playlist {
        /// The playlist URL or ID
        url: String,
        /// Play in shuffle mode
        #[arg(short, long)]
        shuffle: bool,
    },
}

#[derive(Subcommand)]
enum CacheCommands {
    /// List all cached tracks
    List,
    /// Clear the entire cache
    Clear,
}

#[derive(Debug, PartialEq, Eq)]
enum PlaybackControl {
    Finished,
    Next,
    Prev,
    Quit,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct RawModeGuard;

impl RawModeGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        disable_raw_mode().ok();
        println!(); // Ensure we end with a newline
    }
}

fn clear_screen() {
    print!("\x1b[2J\x1b[1;1H");
    std::io::stdout().flush().ok();
}

fn press_enter_to_continue() {
    print!("\n  Press Enter to continue...");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let mins = secs / 60;
    let secs = secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        s.chars().take(max_chars - 3).collect::<String>() + "..."
    } else {
        s.to_string()
    }
}

fn draw_progress_bar(
    elapsed: Duration,
    total: Option<Duration>,
    volume: f32,
    is_paused: bool,
    visualizer: Option<&crate::audio::VisualizerShared>,
) {
    let elapsed_str = format_duration(elapsed);
    let total_str = match total {
        Some(t) => format_duration(t),
        None => "??:??".to_string(),
    };
    
    let bar_width = 30;
    let mut filled = 0;
    if let Some(t) = total {
        if t.as_secs() > 0 {
            filled = ((elapsed.as_secs_f64() / t.as_secs_f64()) * bar_width as f64).round() as usize;
            filled = filled.min(bar_width);
        }
    }

    // Filled portion in purple
    let filled_str = std::iter::repeat("━").take(filled).collect::<String>();
    let filled_styled = theme::style_accent(&filled_str);

    // Indicator (thumb)
    let thumb = if is_paused { "⏸" } else { "◉" };
    let thumb_styled = theme::style_primary(thumb);

    // Unfilled portion in dark slate/blue
    let unfilled_len = bar_width.saturating_sub(filled).saturating_sub(1);
    let unfilled_str = std::iter::repeat("─").take(unfilled_len).collect::<String>();
    let unfilled_styled = theme::style_secondary(&unfilled_str);

    let state_icon = if is_paused { "⏸ PAUSED" } else { "▶ PLAYING" };
    let state_style = if is_paused {
        theme::style_dim_style()
    } else {
        theme::style_primary_style()
    };
    let state_styled = state_style.style(state_icon);

    let vol_pct = (volume * 100.0).round() as i32;
    let vol_icon = if vol_pct == 0 {
        "🔇"
    } else if vol_pct < 50 {
        "🔈"
    } else if vol_pct < 100 {
        "🔉"
    } else {
        "🔊"
    };
    let vol_str = format!("{} {}%", vol_icon, vol_pct);
    let vol_styled = theme::style_accent(&vol_str);

    let time_str = format!("{} / {}", elapsed_str, total_str);
    let time_styled = theme::style_dim(&time_str);

    // Interpolate visualizer bands (8 bands -> 40 columns)
    let num_columns = 40;
    let mut columns = vec![0.0f32; num_columns];
    if let Some(vis) = visualizer {
        if !is_paused {
            for col in 0..num_columns {
                let band_idx_raw = (col as f32 / (num_columns - 1) as f32) * 7.0;
                let band_left = band_idx_raw.floor() as usize;
                let band_right = band_idx_raw.ceil() as usize;
                let t = band_idx_raw - band_left as f32;

                let val_left = vis.get_band(band_left);
                let val_right = vis.get_band(band_right);
                let val = val_left + t * (val_right - val_left);
                columns[col] = val;
            }
        }
    }

    let blocks = [' ', ' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    
    // We will build 5 rows of the visualizer (top row index 0 to bottom row index 4)
    let num_rows = 5;
    let mut visualizer_rows = vec![String::new(); num_rows];
    let left_padding = 15;

    if visualizer.is_some() {
        for row_idx in (0..num_rows).rev() {
            let mut row_str = String::new();
            for col in 0..num_columns {
                let val = columns[col];
                // Apply a compressed gain using square root (boosts quiet sections, reduces clipping on peaks)
                let col_factor = col as f32 / (num_columns - 1) as f32;
                let gain = 1.5 + col_factor * 1.5;
                let compressed = val.sqrt();
                let scaled_val = (compressed * gain).clamp(0.0, 1.0);

                // Row math: 5 levels of height, row_idx is 0 (bottom) to 4 (top)
                let h = (scaled_val * num_rows as f32 - row_idx as f32).clamp(0.0, 1.0);
                let block_idx = (h * 8.0).round() as usize;
                let block_char = blocks[block_idx];

                let col_ratio = col as f32 / (num_columns - 1) as f32;
                let (r, g, b) = if col_ratio < 0.5 {
                    let t = col_ratio * 2.0;
                    (
                        (theme::COLOR_SECONDARY.0 as f32 + t * (theme::COLOR_PRIMARY.0 as f32 - theme::COLOR_SECONDARY.0 as f32)).round() as u8,
                        (theme::COLOR_SECONDARY.1 as f32 + t * (theme::COLOR_PRIMARY.1 as f32 - theme::COLOR_SECONDARY.1 as f32)).round() as u8,
                        (theme::COLOR_SECONDARY.2 as f32 + t * (theme::COLOR_PRIMARY.2 as f32 - theme::COLOR_SECONDARY.2 as f32)).round() as u8,
                    )
                } else {
                    let t = (col_ratio - 0.5) * 2.0;
                    (
                        (theme::COLOR_PRIMARY.0 as f32 + t * (theme::COLOR_ACCENT.0 as f32 - theme::COLOR_PRIMARY.0 as f32)).round() as u8,
                        (theme::COLOR_PRIMARY.1 as f32 + t * (theme::COLOR_ACCENT.1 as f32 - theme::COLOR_PRIMARY.1 as f32)).round() as u8,
                        (theme::COLOR_PRIMARY.2 as f32 + t * (theme::COLOR_ACCENT.2 as f32 - theme::COLOR_PRIMARY.2 as f32)).round() as u8,
                    )
                };

                let styled_char = block_char.to_string().color(owo_colors::Rgb(r, g, b)).to_string();
                row_str.push_str(&styled_char);
            }
            visualizer_rows[num_rows - 1 - row_idx] = row_str;
        }
    }

    // Carriage return and clear line for progress bar
    print!(
        "\r\x1b[K  {}  [ {}{}{} ]  {}  {}",
        state_styled,
        filled_styled,
        thumb_styled,
        unfilled_styled,
        time_styled,
        vol_styled
    );

    if visualizer.is_some() {
        // 1. Print two blank spacer lines to put it lower
        print!("\n\r\x1b[K\n\r\x1b[K");
        
        // 2. Print the five equalizer rows
        for row_str in &visualizer_rows {
            print!(
                "\n\r\x1b[K{:indent$}{}",
                "",
                row_str,
                indent = left_padding
            );
        }
        // Cursor back up 7 lines (2 spacers + 5 rows) and carriage return
        print!("\x1b[7A\r");
    }
    
    std::io::stdout().flush().ok();
}


/// Print a numbered table of tracks.
fn print_track_table(tracks: &[TrackInfo]) {
    let mut table = theme::create_styled_table();
    table.set_header(vec![
        theme::style_header_cell("#"),
        theme::style_header_cell("Title"),
        theme::style_header_cell("Artist"),
        theme::style_header_cell("Duration"),
    ]);

    for (i, track) in tracks.iter().enumerate() {
        let dur_str = match track.duration_secs {
            Some(d) => format_duration(Duration::from_secs(d as u64)),
            None => "--:--".to_string(),
        };
        table.add_row(vec![
            theme::style_data_cell(&(i + 1).to_string(), false),
            theme::style_data_cell(&track.title, true),
            theme::style_data_cell(&track.artist, false),
            theme::style_data_cell(&dur_str, false),
        ]);
    }
    println!("{}", table);
}

fn shuffle_tracks(tracks: &mut [TrackInfo]) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Simple LCG parameters (Numerical Recipes)
    let a: u64 = 1664525;
    let c: u64 = 1013904223;
    let m: u64 = 2u64.pow(32);

    let mut next_random = move || {
        seed = (a.wrapping_mul(seed).wrapping_add(seed.rotate_left(11)).wrapping_add(c)) % m;
        seed
    };

    let n = tracks.len();
    for i in (1..n).rev() {
        let j = (next_random() as usize) % (i + 1);
        tracks.swap(i, j);
    }
}

// ---------------------------------------------------------------------------
// Core playback – reused by both subcommands and interactive menu
// ---------------------------------------------------------------------------

async fn play_track(
    track: &TrackInfo,
    config: &Config,
    db: &Db,
    queue_info: Option<(usize, usize)>,
    current_volume: &mut f32,
    debug: bool,
) -> Result<PlaybackControl> {
    let video_id = &track.id;
    let title = &track.title;
    let artist = &track.artist;
    let total_duration = track.duration_secs.map(|d| Duration::from_secs(d as u64));

    let cache_file = config.cache_dir.join(format!("{}.ogg", video_id));
    let player = AudioPlayer::new()?;

    let cached = db.get_cached_track(video_id)?;
    let use_cache = if let Some(ref c) = cached {
        let path = Path::new(&c.file_path);
        path.exists() && path.extension().map(|ext| ext == "ogg").unwrap_or(false)
    } else {
        false
    };

    // Keep active playback details fresh on a cleared screen
    clear_screen();

    let (sink, visualizer_shared) = if use_cache {
        let track_name = format!("{} - {}", title, artist);
        if let Some((idx, total)) = queue_info {
            println!("  💿 [{}/{}] Playing (cached): {}", idx + 1, total, theme::style_primary(&track_name));
        } else {
            println!("  💿 Playing (cached): {}", theme::style_primary(&track_name));
        }
        let (sink, _total_dur, vis) = player.play_local(cache_file.clone())?;
        sink.set_volume(*current_volume);
        (sink, vis)
    } else {
        // Remove any partial/corrupted cache file on disk
        if cache_file.exists() {
            std::fs::remove_file(&cache_file).ok();
        }

        let track_name = format!("{} - {}", title, artist);
        let pb = if !debug {
            let prefix = if let Some((idx, total)) = queue_info {
                format!("⏳ [{}/{}]", idx + 1, total)
            } else {
                "⏳".to_string()
            };
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_style(
                indicatif::ProgressStyle::default_spinner()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                    .template(&format!("  {} {{spinner}} Buffering: {{msg}}", prefix))
                    .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
            );
            spinner.set_message(theme::style_primary(&track_name).to_string());
            spinner.enable_steady_tick(Duration::from_millis(80));
            Some(spinner)
        } else {
            if let Some((idx, total)) = queue_info {
                println!("  ⏳ [{}/{}] Buffering: {}", idx + 1, total, theme::style_primary(&track_name));
            } else {
                println!("  ⏳ Buffering: {}", theme::style_primary(&track_name));
            }
            None
        };

        let yt_dlp_path = config.ensure_yt_dlp().await?;

        // Add cookies to download if logged in
        let mut cmd = tokio::process::Command::new(&yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes", &config.get_js_runtime_arg(),
            "--remote-components", "ejs:github",
            "-x",
            "--audio-format", "vorbis",
            &format!("https://www.youtube.com/watch?v={}", video_id),
            "-o", &cache_file.to_string_lossy(),
        ]);
        if config.cookies_path.exists() && std::fs::metadata(&config.cookies_path).map(|m| m.len() > 0).unwrap_or(false) {
            cmd.arg("--cookies").arg(&config.cookies_path);
        }
        cmd.kill_on_drop(true);

        let status = if debug {
            cmd.status().await?
        } else {
            let output = cmd.output().await?;
            log::debug!("yt-dlp stdout: {}", String::from_utf8_lossy(&output.stdout));
            log::debug!("yt-dlp stderr: {}", String::from_utf8_lossy(&output.stderr));
            output.status
        };

        if let Some(ref spinner) = pb {
            spinner.finish_and_clear();
        }

        if !status.success() {
            let err_msg = "Failed to download audio track using yt-dlp";
            println!("  ❌ {}", theme::style_error(err_msg));
            press_enter_to_continue();
            return Err(anyhow::anyhow!(err_msg));
        }

        // Register cached track in DB
        let duration_u32 = total_duration.map(|d| d.as_secs() as u32).unwrap_or(0);
        let file_path_str = cache_file.to_string_lossy().to_string();
        if let Err(e) = db.insert_cached_track(video_id, title, artist, duration_u32, &file_path_str) {
            eprintln!("  Failed to register cached track in DB: {}", e);
        }

        let (sink, _total_dur, vis) = player.play_local(cache_file.clone())?;
        sink.set_volume(*current_volume);
        (sink, vis)
    };

    db.add_history(video_id, title, artist)?;

    let controls_help = if queue_info.is_some() {
        "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [N] Next  [P] Prev  [Q] Stop/Back"
    } else {
        "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [Q] Stop/Back"
    };
    println!("{}", theme::style_dim(controls_help));

    let mut control = PlaybackControl::Finished;

    // Scope for raw mode guard
    {
        let _guard = RawModeGuard::new()?;
        let mut last_tick = Instant::now();
        let mut elapsed = Duration::from_secs(0);
        let mut last_seek = Instant::now() - Duration::from_secs(1);

        while !sink.empty() {
            let now = Instant::now();
            let delta = now.duration_since(last_tick);
            last_tick = now;

            if !sink.is_paused() {
                elapsed += delta;
            }

            draw_progress_bar(elapsed, total_duration, sink.volume(), sink.is_paused(), Some(&visualizer_shared));

            if event::poll(Duration::from_millis(40))? {
                if let Event::Key(key_event) = event::read()? {
                    if key_event.kind == event::KeyEventKind::Press || key_event.kind == event::KeyEventKind::Repeat {
                        match key_event.code {
                            KeyCode::Char(' ') => {
                                if sink.is_paused() {
                                    sink.play();
                                } else {
                                    sink.pause();
                                }
                            }
                            KeyCode::Char('q') | KeyCode::Esc => {
                                sink.stop();
                                control = PlaybackControl::Quit;
                                break;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                if queue_info.is_some() {
                                    sink.stop();
                                    control = PlaybackControl::Next;
                                    break;
                                }
                            }
                            KeyCode::Char('p') | KeyCode::Char('P') => {
                                if queue_info.is_some() {
                                    sink.stop();
                                    control = PlaybackControl::Prev;
                                    break;
                                }
                            }
                            KeyCode::Left => {
                                let now_seek = Instant::now();
                                if now_seek.duration_since(last_seek) >= Duration::from_millis(250) {
                                    let new_pos = elapsed.saturating_sub(Duration::from_secs(5));
                                    if sink.try_seek(new_pos).is_ok() {
                                        elapsed = new_pos;
                                        last_seek = now_seek;
                                    }
                                }
                            }
                            KeyCode::Right => {
                                let now_seek = Instant::now();
                                if now_seek.duration_since(last_seek) >= Duration::from_millis(250) {
                                    let new_pos = elapsed + Duration::from_secs(5);
                                    let can_seek = match total_duration {
                                        Some(total_dur) => new_pos < total_dur,
                                        None => true,
                                    };
                                    if can_seek {
                                        if sink.try_seek(new_pos).is_ok() {
                                            elapsed = new_pos;
                                            last_seek = now_seek;
                                        }
                                    }
                                }
                            }
                            KeyCode::Up => {
                                let vol = (sink.volume() + 0.1).min(2.0);
                                sink.set_volume(vol);
                                *current_volume = vol;
                            }
                            KeyCode::Down => {
                                let vol = (sink.volume() - 0.1).max(0.0);
                                sink.set_volume(vol);
                                *current_volume = vol;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        // Clear all 7 visualizer lines and the progress bar line when playback stops/finishes
        for _ in 0..7 {
            print!("\n\r\x1b[K");
        }
        print!("\x1b[7A\r\x1b[K");
        std::io::stdout().flush().ok();
    }

    Ok(control)
}

fn spawn_prefetch(
    track: TrackInfo,
    config: Config,
    db: Db,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let video_id = &track.id;
        let cache_file = config.cache_dir.join(format!("{}.ogg", video_id));

        // Check if already exists on disk
        if cache_file.exists() {
            return;
        }

        // Run yt-dlp to download in background
        if let Ok(yt_dlp_path) = config.ensure_yt_dlp().await {
            let mut cmd = tokio::process::Command::new(&yt_dlp_path);
            cmd.args(&[
                "--no-warnings",
                "--js-runtimes", &config.get_js_runtime_arg(),
                "--remote-components", "ejs:github",
                "-x",
                "--audio-format", "vorbis",
                &format!("https://www.youtube.com/watch?v={}", video_id),
                "-o", &cache_file.to_string_lossy(),
            ]);
            if config.cookies_path.exists() && std::fs::metadata(&config.cookies_path).map(|m| m.len() > 0).unwrap_or(false) {
                cmd.arg("--cookies").arg(&config.cookies_path);
            }
            cmd.kill_on_drop(true);

            if let Ok(output) = cmd.output().await {
                if output.status.success() {
                    let duration_u32 = track.duration_secs.unwrap_or(0);
                    let file_path_str = cache_file.to_string_lossy().to_string();
                    let _ = db.insert_cached_track(video_id, &track.title, &track.artist, duration_u32, &file_path_str);
                }
            }
        }
    })
}

async fn play_queue(
    tracks: Vec<TrackInfo>,
    start_idx: usize,
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    autoplay_ctoken: Option<(String, ContinuationEndpoint)>,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    if tracks.is_empty() { return Ok(()); }
    let mut tracks = tracks;
    let mut autoplay_ctoken = autoplay_ctoken;
    let mut idx = start_idx;
    let mut active_prefetch: Option<(usize, tokio::task::JoinHandle<()>)> = None;

    loop {
        // Fetch more tracks if we are near the end of the autoplay queue
        if let Some((ctoken, endpoint)) = autoplay_ctoken.clone() {
            if idx + 5 >= tracks.len() {
                if !debug {
                    print!("\r\x1b[K  📻 Fetching more autoplay recommendations...");
                    std::io::stdout().flush().ok();
                } else {
                    println!("  📻 Fetching more autoplay recommendations...");
                }
                match client.fetch_next_autoplay_page(&ctoken, endpoint).await {
                    Ok((mut new_tracks, new_ctoken)) => {
                        tracks.append(&mut new_tracks);
                        if let Some(tok) = new_ctoken {
                            autoplay_ctoken = Some((tok, endpoint));
                        } else {
                            autoplay_ctoken = None;
                        }
                        // Clear the "fetching" line
                        if !debug {
                            print!("\r\x1b[K");
                            std::io::stdout().flush().ok();
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to fetch next autoplay page: {}", e);
                        autoplay_ctoken = None;
                    }
                }
            }
        }

        // Manage prefetch for next track
        if let Some((pref_idx, ref handle)) = active_prefetch {
            if pref_idx != idx + 1 {
                handle.abort();
                active_prefetch = None;
            }
        }

        if active_prefetch.is_none() && idx + 1 < tracks.len() {
            let handle = spawn_prefetch(tracks[idx + 1].clone(), config.clone(), db.clone());
            active_prefetch = Some((idx + 1, handle));
        }

        let track = &tracks[idx];
        let control = play_track(track, config, db, Some((idx, tracks.len())), current_volume, debug).await?;
        match control {
            PlaybackControl::Finished | PlaybackControl::Next => {
                idx += 1;
                if idx >= tracks.len() {
                    println!("\n  Queue finished.");
                    press_enter_to_continue();
                    break;
                }
            }
            PlaybackControl::Prev => {
                if idx > 0 {
                    idx -= 1;
                } else {
                    println!("\n  Already at the first track.");
                    press_enter_to_continue();
                }
            }
            PlaybackControl::Quit => {
                if let Some((_, handle)) = active_prefetch {
                    handle.abort();
                }
                break;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive menu flows
// ---------------------------------------------------------------------------

fn interact_table_select(
    prompt: &str,
    headers: Vec<&str>,
    tracks: &[TrackInfo],
    go_back_label: &str,
) -> Result<Option<usize>> {
    let mut selected_idx = 0;
    let num_options = tracks.len() + 1;

    let _guard = RawModeGuard::new()?;

    loop {
        clear_screen();
        print!("  ❓ {}\r\n\r\n", theme::style_primary(prompt));

        let mut table = theme::create_styled_table();
        table.set_header(headers.iter().map(|h| theme::style_header_cell(h)).collect::<Vec<_>>());

        for (i, track) in tracks.iter().enumerate() {
            let is_selected = i == selected_idx;
            let dur_str = match track.duration_secs {
                Some(d) => format_duration(Duration::from_secs(d as u64)),
                None => "--:--".to_string(),
            };

            let num_cell = if is_selected {
                comfy_table::Cell::new(format!("> {}", i + 1))
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(format!("  {}", i + 1))
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let title_cell = if is_selected {
                comfy_table::Cell::new(&track.title)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_ACCENT.0, g: theme::COLOR_ACCENT.1, b: theme::COLOR_ACCENT.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&track.title)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let artist_cell = if is_selected {
                comfy_table::Cell::new(&track.artist)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&track.artist)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let dur_cell = if is_selected {
                comfy_table::Cell::new(dur_str)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(dur_str)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            table.add_row(vec![num_cell, title_cell, artist_cell, dur_cell]);
        }

        // Add Go Back row
        let is_selected = selected_idx == tracks.len();
        let num_cell = if is_selected {
            comfy_table::Cell::new("> 🔙")
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new("  🔙")
        };
        let label_cell = if is_selected {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
        };
        table.add_row(vec![
            num_cell,
            label_cell,
            comfy_table::Cell::new(""),
            comfy_table::Cell::new(""),
        ]);

        for line in table.to_string().lines() {
            print!("  {}\r\n", line);
        }

        print!("\r\n  {}\r\n", theme::style_dim("🎮 Controls: [↑/↓] Move  [Enter] Select  [Q/Esc] Back"));
        std::io::stdout().flush().ok();

        if let Event::Key(key_event) = event::read()? {
            if key_event.kind == event::KeyEventKind::Press || key_event.kind == event::KeyEventKind::Repeat {
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('p') | KeyCode::Char('P') => {
                        selected_idx = (selected_idx + num_options - 1) % num_options;
                    }
                    KeyCode::Down | KeyCode::Char('n') | KeyCode::Char('N') => {
                        selected_idx = (selected_idx + 1) % num_options;
                    }
                    KeyCode::Enter => {
                        if selected_idx == tracks.len() {
                            return Ok(None);
                        } else {
                            return Ok(Some(selected_idx));
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        return Ok(None);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn interact_history_select(
    prompt: &str,
    headers: Vec<&str>,
    history: &[crate::db::HistoryEntry],
    go_back_label: &str,
) -> Result<Option<usize>> {
    let mut selected_idx = 0;
    let num_options = history.len() + 1;

    let _guard = RawModeGuard::new()?;

    loop {
        clear_screen();
        print!("  ❓ {}\r\n\r\n", theme::style_primary(prompt));

        let mut table = theme::create_styled_table();
        table.set_header(headers.iter().map(|h| theme::style_header_cell(h)).collect::<Vec<_>>());

        for (i, entry) in history.iter().enumerate() {
            let is_selected = i == selected_idx;
            let short_date = if entry.played_at.len() > 19 {
                &entry.played_at[..19]
            } else {
                &entry.played_at
            };

            let num_cell = if is_selected {
                comfy_table::Cell::new(format!("> {}", i + 1))
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(format!("  {}", i + 1))
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let title_cell = if is_selected {
                comfy_table::Cell::new(&entry.title)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_ACCENT.0, g: theme::COLOR_ACCENT.1, b: theme::COLOR_ACCENT.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&entry.title)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let artist_cell = if is_selected {
                comfy_table::Cell::new(&entry.artist)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&entry.artist)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            let date_cell = if is_selected {
                comfy_table::Cell::new(short_date)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(short_date)
                    .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
            };

            table.add_row(vec![num_cell, title_cell, artist_cell, date_cell]);
        }

        // Add Go Back row
        let is_selected = selected_idx == history.len();
        let num_cell = if is_selected {
            comfy_table::Cell::new("> 🔙")
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new("  🔙")
        };
        let label_cell = if is_selected {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb { r: theme::COLOR_PRIMARY.0, g: theme::COLOR_PRIMARY.1, b: theme::COLOR_PRIMARY.2 })
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb { r: theme::COLOR_DIM.0, g: theme::COLOR_DIM.1, b: theme::COLOR_DIM.2 })
        };
        table.add_row(vec![
            num_cell,
            label_cell,
            comfy_table::Cell::new(""),
            comfy_table::Cell::new(""),
        ]);

        for line in table.to_string().lines() {
            print!("  {}\r\n", line);
        }

        print!("\r\n  {}\r\n", theme::style_dim("🎮 Controls: [↑/↓] Move  [Enter] Select  [Q/Esc] Back"));
        std::io::stdout().flush().ok();

        if let Event::Key(key_event) = event::read()? {
            if key_event.kind == event::KeyEventKind::Press || key_event.kind == event::KeyEventKind::Repeat {
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('p') | KeyCode::Char('P') => {
                        selected_idx = (selected_idx + num_options - 1) % num_options;
                    }
                    KeyCode::Down | KeyCode::Char('n') | KeyCode::Char('N') => {
                        selected_idx = (selected_idx + 1) % num_options;
                    }
                    KeyCode::Enter => {
                        if selected_idx == history.len() {
                            return Ok(None);
                        } else {
                            return Ok(Some(selected_idx));
                        }
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        return Ok(None);
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn run_search_and_play(config: &Config, db: &Db, client: &NetworkClient, current_volume: &mut f32, debug: bool) -> Result<()> {
    loop {
        clear_screen();
        let query: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Search YouTube Music (or 'q' to go back)")
            .allow_empty(true)
            .interact_text()?;

        let query = query.trim();
        if query.is_empty() { continue; }
        if query.eq_ignore_ascii_case("q") { break; }

        println!("\n  🔍 Searching for '{}'...", theme::style_primary(query));
        let tracks = client.search(query).await?;

        if tracks.is_empty() {
            println!("  ❌ {}", theme::style_error("No tracks found."));
            press_enter_to_continue();
            continue;
        }

        loop {
            let selection = match interact_table_select(
                "Select track to play",
                vec!["#", "Title", "Artist", "Duration"],
                &tracks,
                "Search again / Go back",
            )? {
                Some(idx) => idx,
                None => break,
            };

            let selected_track = tracks[selection].clone();
            println!("\n  📻 Fetching autoplay recommendations...");
            let mut queue = vec![selected_track.clone()];
            let mut autoplay_ctoken = None;
            
            match client.fetch_autoplay_queue(&selected_track.id).await {
                Ok((mut autoplay_tracks, ctoken)) => {
                    queue.append(&mut autoplay_tracks);
                    autoplay_ctoken = ctoken;
                }
                Err(e) => {
                    log::warn!("Failed to fetch autoplay tracks: {}", e);
                }
            }

            let _ = play_queue(queue, 0, config, db, client, autoplay_ctoken, current_volume, debug).await?;
        }
    }
    Ok(())
}

async fn run_history(config: &Config, db: &Db, client: &NetworkClient, current_volume: &mut f32, debug: bool) -> Result<()> {
    loop {
        clear_screen();
        let history = db.get_history(20)?;

        if history.is_empty() {
            println!("  No playback history found.");
            press_enter_to_continue();
            return Ok(());
        }

        let selection = match interact_history_select(
            "Select track to replay",
            vec!["#", "Title", "Artist", "Played At"],
            &history,
            "Go back",
        )? {
            Some(idx) => idx,
            None => break,
        };

        let entry = &history[selection];
        let track = TrackInfo {
            id: entry.video_id.clone(),
            title: entry.title.clone(),
            artist: entry.artist.clone(),
            duration_secs: None,
        };
        println!("\n  📻 Fetching autoplay recommendations...");
        let mut queue = vec![track.clone()];
        let mut autoplay_ctoken = None;
        
        match client.fetch_autoplay_queue(&track.id).await {
            Ok((mut autoplay_tracks, ctoken)) => {
                queue.append(&mut autoplay_tracks);
                autoplay_ctoken = ctoken;
            }
            Err(e) => {
                log::warn!("Failed to fetch autoplay tracks: {}", e);
            }
        }
        play_queue(queue, 0, config, db, client, autoplay_ctoken, current_volume, debug).await?;
    }
    Ok(())
}

async fn run_cache_menu(_config: &Config, db: &Db) -> Result<()> {
    loop {
        clear_screen();
        let selections = &[
            "💾 List cached tracks",
            "🗑️ Clear cache",
            "🔙 Go back"
        ];
        
        let selection = dialoguer::Select::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Cache Management")
            .default(0)
            .items(&selections[..])
            .interact()?;

        match selection {
            0 => {
                let tracks = db.list_cached_tracks()?;
                if tracks.is_empty() {
                    println!("  No cached tracks found.");
                } else {
                    let mut table = theme::create_styled_table();
                    table.set_header(vec![
                        theme::style_header_cell("#"),
                        theme::style_header_cell("Title"),
                        theme::style_header_cell("Artist"),
                        theme::style_header_cell("Duration"),
                        theme::style_header_cell("Cached At"),
                    ]);
                    for (i, track) in tracks.iter().enumerate() {
                        let dur_str = format_duration(Duration::from_secs(track.duration_secs as u64));
                        let short_date = if track.cached_at.len() > 19 {
                            &track.cached_at[..19]
                        } else {
                            &track.cached_at
                        };
                        table.add_row(vec![
                            theme::style_data_cell(&(i + 1).to_string(), false),
                            theme::style_data_cell(&track.title, true),
                            theme::style_data_cell(&track.artist, false),
                            theme::style_data_cell(&dur_str, false),
                            theme::style_data_cell(short_date, false),
                        ]);
                    }
                    println!("{}", table);
                }
                press_enter_to_continue();
            }
            1 => {
                let tracks = db.list_cached_tracks()?;
                let count = tracks.len();
                
                if count == 0 {
                    println!("  No cached tracks to clear.");
                    press_enter_to_continue();
                    continue;
                }

                let confirm = dialoguer::Confirm::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt(format!("Are you sure you want to delete {} cached track(s)?", count))
                    .default(false)
                    .interact()?;
                
                if confirm {
                    for track in tracks {
                        let path = PathBuf::from(&track.file_path);
                        if path.exists() {
                            std::fs::remove_file(path).ok();
                        }
                    }
                    db.clear_cache()?;
                    println!("  ✨ {}", theme::style_primary(&format!("Cleared {} cached track(s).", count)));
                } else {
                    println!("  Cancelled.");
                }
                press_enter_to_continue();
            }
            _ => break,
        }
    }
    Ok(())
}

async fn run_login_flow(config: &Config) -> Result<()> {
    let browsers = &[
        "firefox",
        "chrome",
        "chromium",
        "brave",
        "edge",
        "opera",
        "safari",
        "vivaldi",
        "🔙 Cancel",
    ];

    let selection = dialoguer::Select::with_theme(&theme::get_dialoguer_theme())
        .with_prompt("Select browser to extract cookies from")
        .default(0)
        .items(&browsers[..])
        .interact()?;

    if selection == browsers.len() - 1 {
        return Ok(());
    }

    let browser = browsers[selection];
    println!("  Extracting cookies from {}...", theme::style_primary(browser));
    if let Err(e) = config.login(browser).await {
        println!("  ❌ {}", theme::style_error(&e.to_string()));
    } else {
        println!("  ✨ {}", theme::style_primary("Logged in successfully!"));
    }
    press_enter_to_continue();
    Ok(())
}

async fn run_playlist_playback(
    playlist_id: &str,
    playlist_title: &str,
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    let yt_dlp_path = config.ensure_yt_dlp().await?;

    let url = if playlist_id == "LM" {
        "https://music.youtube.com/playlist?list=LM".to_string()
    } else if playlist_id.starts_with("PL") || playlist_id.starts_with("RD") {
        format!("https://music.youtube.com/playlist?list={}", playlist_id)
    } else if playlist_id.starts_with("http") {
        playlist_id.to_string()
    } else {
        format!("https://music.youtube.com/playlist?list={}", playlist_id)
    };

    println!("\n  Fetching playlist tracks...");
    let tracks = match client.fetch_playlist(&yt_dlp_path, &config.cookies_path, &config.get_js_runtime_arg(), &url).await {
        Ok(t) => t,
        Err(e) => {
            println!("  ❌ Failed to fetch playlist tracks: {}", e);
            press_enter_to_continue();
            return Ok(());
        }
    };

    if tracks.is_empty() {
        println!("  No tracks found in playlist '{}'.", playlist_title);
        press_enter_to_continue();
        return Ok(());
    }

    loop {
        clear_screen();
        println!("\n  ── Playlist: {} ──", theme::style_primary(playlist_title));
        print_track_table(&tracks);

        let mut options = vec![
            "▶️ Play All".to_string(),
            "🔀 Shuffle Play".to_string(),
        ];
        for (i, t) in tracks.iter().enumerate() {
            let dur = match t.duration_secs {
                Some(d) => format_duration(Duration::from_secs(d as u64)),
                None => "--:--".to_string(),
            };
            options.push(format!("{}. {} - {} [{}]", i + 1, t.title, t.artist, dur));
        }
        options.push("🔙 Go back".to_string());

        let selection = dialoguer::FuzzySelect::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Choose playback option or search track")
            .default(0)
            .items(&options)
            .interact()?;

        if selection == 0 {
            play_queue(tracks.clone(), 0, config, db, client, None, current_volume, debug).await?;
        } else if selection == 1 {
            let mut shuffled_tracks = tracks.clone();
            shuffle_tracks(&mut shuffled_tracks);
            play_queue(shuffled_tracks, 0, config, db, client, None, current_volume, debug).await?;
        } else if selection == options.len() - 1 {
            break;
        } else {
            let track_idx = selection - 2;
            play_queue(tracks.clone(), track_idx, config, db, client, None, current_volume, debug).await?;
        }
    }
    Ok(())
}

async fn run_library_playlists(config: &Config, db: &Db, client: &NetworkClient, current_volume: &mut f32, debug: bool) -> Result<()> {
    loop {
        clear_screen();
        let yt_dlp_path = config.ensure_yt_dlp().await?;

        println!("\n  Fetching your playlists...");
        let playlists = match client.fetch_library_playlists(&yt_dlp_path, &config.cookies_path, &config.get_js_runtime_arg()).await {
            Ok(p) => p,
            Err(e) => {
                println!("  ❌ Failed to fetch library playlists: {}", e);
                press_enter_to_continue();
                return Ok(());
            }
        };

        if playlists.is_empty() {
            println!("  No playlists found in your library.");
            press_enter_to_continue();
            return Ok(());
        }

        let mut options: Vec<String> = playlists.iter().map(|p| {
            let count_str = match p.track_count {
                Some(c) => format!(" ({} tracks)", c),
                None => "".to_string(),
            };
            format!("📁 {}{}", p.title, count_str)
        }).collect();
        options.push("🔙 Go back".to_string());

        let selection = dialoguer::Select::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Select a playlist")
            .default(0)
            .items(&options)
            .interact()?;

        if selection == playlists.len() {
            break;
        }

        let playlist = &playlists[selection];
        run_playlist_playback(&playlist.id, &playlist.title, config, db, client, current_volume, debug).await?;
    }
    Ok(())
}

async fn run_interactive_menu(config: &Config, db: &Db, client: &NetworkClient, current_volume: &mut f32, debug: bool) -> Result<()> {
    loop {
        clear_screen();
        theme::print_banner();
        let logged_in = config.is_logged_in();
        
        let mut selections = vec![
            "🔍 Search and Play".to_string(),
            "📜 Playback History".to_string(),
            "💾 Cache Management".to_string(),
        ];
        if logged_in {
            selections.push("🎵 My Playlists".to_string());
            selections.push("🔑 Logout".to_string());
        } else {
            selections.push("🔑 Login".to_string());
        }
        selections.push("🚪 Exit".to_string());

        let selection = dialoguer::Select::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Main Menu")
            .default(0)
            .items(&selections)
            .interact()?;

        match selection {
            0 => run_search_and_play(config, db, client, current_volume, debug).await?,
            1 => run_history(config, db, client, current_volume, debug).await?,
            2 => run_cache_menu(config, db).await?,
            3 => {
                if logged_in {
                    run_library_playlists(config, db, client, current_volume, debug).await?;
                } else {
                    run_login_flow(config).await?;
                }
            }
            4 => {
                if logged_in {
                    config.logout()?;
                    println!("  ✨ Logged out successfully.");
                    press_enter_to_continue();
                } else {
                    println!("  Goodbye! 👋");
                    break;
                }
            }
            5 => {
                println!("  Goodbye! 👋");
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::new();

    // Initialize file logger
    let log_path = config.config_dir.join("ytm-cli.log");
    if let Ok(file) = std::fs::File::create(log_path) {
        simplelog::WriteLogger::init(
            simplelog::LevelFilter::Debug,
            simplelog::Config::default(),
            file,
        ).ok();
    }

    let db = Db::new(&config.db_path)?;
    let client = NetworkClient::new();
    let mut current_volume = 1.0f32;
    let debug = cli.debug;

    match cli.command {
        // No subcommand provided → launch the interactive menu
        None => {
            run_interactive_menu(&config, &db, &client, &mut current_volume, debug).await?;
        }
        // Subcommands preserved for scripting / one-shot usage
        Some(Commands::Play { query }) => {
            println!("Searching for '{}'...", query);
            let tracks = client.search(&query).await?;
            if tracks.is_empty() {
                println!("No matching track found.");
                return Ok(());
            }
            let selected_track = tracks[0].clone();
            println!("  📻 Fetching autoplay recommendations... ");
            let mut queue = vec![selected_track.clone()];
            let mut autoplay_ctoken = None;
            
            match client.fetch_autoplay_queue(&selected_track.id).await {
                Ok((mut autoplay_tracks, ctoken)) => {
                    queue.append(&mut autoplay_tracks);
                    autoplay_ctoken = ctoken;
                }
                Err(e) => {
                    log::warn!("Failed to fetch autoplay tracks: {}", e);
                }
            }
            play_queue(queue, 0, &config, &db, &client, autoplay_ctoken, &mut current_volume, debug).await?;
        }
        Some(Commands::Search { query }) => {
            println!("Searching for '{}'...", query);
            let tracks = client.search(&query).await?;
            if tracks.is_empty() {
                println!("No tracks found.");
                return Ok(());
            }
            print_track_table(&tracks);
        }
        Some(Commands::Cache { action }) => {
            match action {
                CacheCommands::List => {
                    let tracks = db.list_cached_tracks()?;
                    if tracks.is_empty() {
                        println!("No cached tracks found.");
                        return Ok(());
                    }
                    println!("{:<12} | {:<30} | {:<20} | {:<8} | {}", "ID", "Title", "Artist", "Duration", "Cached At");
                    println!("{}", "-".repeat(90));
                    for track in tracks {
                        let dur_str = format_duration(Duration::from_secs(track.duration_secs as u64));
                        println!("{:<12} | {:<30} | {:<20} | {:<8} | {}", 
                                 track.video_id, 
                                 truncate(&track.title, 30), 
                                 truncate(&track.artist, 20), 
                                 dur_str, 
                                 track.cached_at);
                    }
                }
                CacheCommands::Clear => {
                    let tracks = db.list_cached_tracks()?;
                    for track in tracks {
                        let path = PathBuf::from(&track.file_path);
                        if path.exists() {
                            std::fs::remove_file(path).ok();
                        }
                    }
                    db.clear_cache()?;
                    println!("Cache cleared successfully.");
                }
            }
        }
        Some(Commands::History { limit }) => {
            let history = db.get_history(limit)?;
            if history.is_empty() {
                println!("No playback history found.");
                return Ok(());
            }
            println!("{:<25} | {:<30} | {:<20} | {}", "Played At", "Title", "Artist", "Video ID");
            println!("{}", "-".repeat(90));
            for entry in history {
                println!("{:<25} | {:<30} | {:<20} | {}", 
                         entry.played_at, 
                         truncate(&entry.title, 30), 
                         truncate(&entry.artist, 20), 
                         entry.video_id);
            }
        }
        Some(Commands::Login { browser }) => {
            config.login(&browser).await?;
        }
        Some(Commands::Logout) => {
            config.logout()?;
            println!("Logged out successfully.");
        }
        Some(Commands::Playlist { url, shuffle }) => {
            let yt_dlp_path = config.ensure_yt_dlp().await?;
            println!("Fetching playlist details...");
            let mut tracks = client.fetch_playlist(&yt_dlp_path, &config.cookies_path, &config.get_js_runtime_arg(), &url).await?;
            if tracks.is_empty() {
                println!("No tracks found in playlist.");
                return Ok(());
            }
            if shuffle {
                shuffle_tracks(&mut tracks);
            }
            play_queue(tracks, 0, &config, &db, &client, None, &mut current_volume, debug).await?;
        }
    }
    
    Ok(())
}
