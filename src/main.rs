pub mod audio;
pub mod config;
pub mod db;
pub mod discord;
pub mod discord_rpc;
pub mod lyrics;
pub mod network;
pub mod theme;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use owo_colors::OwoColorize;
use rustypipe::model::paginator::ContinuationEndpoint;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audio::{AudioPlayer, VisualizerShared};
use crate::config::Config;
use crate::db::Db;
use crate::network::{AlbumInfo, NetworkClient, TrackInfo};

#[derive(Parser)]
#[command(
    name = "ytm-cli",
    version = env!("CARGO_PKG_VERSION"),
    about = "YouTube Music CLI client"
)]
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
    /// Search and play an album entirely
    Album {
        /// The search query
        query: String,
    },
    /// Search for tracks matching the query
    Search {
        /// The search query
        query: String,
    },
    /// Search for albums matching the query
    SearchAlbum {
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
        /// The browser to extract cookies from (e.g. firefox, zen, chrome, chromium, brave, edge)
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

async fn fetch_playlist_with_retry(
    config: &Config,
    client: &NetworkClient,
    playlist_url: &str,
) -> Result<Vec<TrackInfo>, anyhow::Error> {
    let yt_dlp_path = config.ensure_yt_dlp().await?;
    let browser = config.get_cookie_browser_arg();
    let js_runtime = config.get_js_runtime_arg();

    let first_try = client
        .fetch_playlist(
            &yt_dlp_path,
            browser.as_deref(),
            &config.cookies_path,
            &js_runtime,
            playlist_url,
        )
        .await;

    if let Err(ref e) = first_try {
        if e.to_string().contains("SESSION_EXPIRED") {
            if let Some(b) = browser {
                let confirm = dialoguer::Confirm::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt(format!(
                        "Your YouTube session has expired. Refresh cookies from {}?",
                        theme::style_primary(&b)
                    ))
                    .default(true)
                    .interact();

                match confirm {
                    Ok(true) => {
                        println!("  🔑 Refreshing cookies...");
                        if let Err(err) = config.login(&b).await {
                            println!("  ❌ Failed to refresh cookies: {}", err);
                            press_enter_to_continue();
                        } else {
                            client.reset_cookies().await;
                            println!("  🔄 Retrying playlist fetch...");
                            return client
                                .fetch_playlist(
                                    &yt_dlp_path,
                                    Some(&b),
                                    &config.cookies_path,
                                    &js_runtime,
                                    playlist_url,
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            } else {
                println!(
                    "  ❌ Session expired, and no browser is configured. Please login using the login feature."
                );
                press_enter_to_continue();
            }
        }
    }

    first_try
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

#[derive(Debug)]
pub enum LyricsState {
    Loading,
    Loaded(crate::lyrics::LyricsData),
    Unavailable,
}

fn draw_progress_bar(
    elapsed: Duration,
    total: Option<Duration>,
    volume: f32,
    is_paused: bool,
    visualizer: Option<&crate::audio::VisualizerShared>,
    lyrics_state: Option<&LyricsState>,
    show_lyrics: bool,
) {
    let elapsed_str = format_duration(elapsed);
    let total_str = match total {
        Some(t) => format_duration(t),
        None => "??:??".to_string(),
    };

    let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80) as usize;
    let bar_width = term_width.saturating_sub(44).clamp(30, 80);
    let mut filled = 0;
    if let Some(t) = total {
        if t.as_secs() > 0 {
            filled =
                ((elapsed.as_secs_f64() / t.as_secs_f64()) * bar_width as f64).round() as usize;
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
    let unfilled_str = std::iter::repeat("─")
        .take(unfilled_len)
        .collect::<String>();
    let unfilled_styled = theme::style_secondary(&unfilled_str);

    let state_icon = if is_paused {
        "⏸ PAUSED"
    } else {
        "▶ PLAYING"
    };
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
                let col_factor = col as f32 / (num_columns - 1) as f32;
                let gain = 1.5 + col_factor * 1.5;
                let compressed = val.sqrt();
                let scaled_val = (compressed * gain).clamp(0.0, 1.0);

                let h = (scaled_val * num_rows as f32 - row_idx as f32).clamp(0.0, 1.0);
                let block_idx = (h * 8.0).round() as usize;
                let block_char = blocks[block_idx];

                let col_ratio = col as f32 / (num_columns - 1) as f32;
                let (r, g, b) = if col_ratio < 0.5 {
                    let t = col_ratio * 2.0;
                    (
                        (theme::COLOR_SECONDARY.0 as f32
                            + t * (theme::COLOR_PRIMARY.0 as f32 - theme::COLOR_SECONDARY.0 as f32))
                            .round() as u8,
                        (theme::COLOR_SECONDARY.1 as f32
                            + t * (theme::COLOR_PRIMARY.1 as f32 - theme::COLOR_SECONDARY.1 as f32))
                            .round() as u8,
                        (theme::COLOR_SECONDARY.2 as f32
                            + t * (theme::COLOR_PRIMARY.2 as f32 - theme::COLOR_SECONDARY.2 as f32))
                            .round() as u8,
                    )
                } else {
                    let t = (col_ratio - 0.5) * 2.0;
                    (
                        (theme::COLOR_PRIMARY.0 as f32
                            + t * (theme::COLOR_ACCENT.0 as f32 - theme::COLOR_PRIMARY.0 as f32))
                            .round() as u8,
                        (theme::COLOR_PRIMARY.1 as f32
                            + t * (theme::COLOR_ACCENT.1 as f32 - theme::COLOR_PRIMARY.1 as f32))
                            .round() as u8,
                        (theme::COLOR_PRIMARY.2 as f32
                            + t * (theme::COLOR_ACCENT.2 as f32 - theme::COLOR_PRIMARY.2 as f32))
                            .round() as u8,
                    )
                };

                let styled_char = block_char
                    .to_string()
                    .color(owo_colors::Rgb(r, g, b))
                    .to_string();
                row_str.push_str(&styled_char);
            }
            visualizer_rows[num_rows - 1 - row_idx] = row_str;
        }
    }

    let mut lines_printed = 0;

    // Carriage return and clear line for progress bar
    print!(
        "\r\x1b[K  {}  [ {}{}{} ]  {}  {}",
        state_styled, filled_styled, thumb_styled, unfilled_styled, time_styled, vol_styled
    );

    if visualizer.is_some() {
        // 1. Print two blank spacer lines to put it lower
        print!("\n\r\x1b[K\n\r\x1b[K");
        lines_printed += 2;

        // 2. Print the five equalizer rows
        for row_str in &visualizer_rows {
            print!("\n\r\x1b[K{:indent$}{}", "", row_str, indent = left_padding);
            lines_printed += 1;
        }
    }

    if show_lyrics {
        if let Some(state) = lyrics_state {
            print!("\n\r\x1b[K");
            lines_printed += 1;

            match state {
                LyricsState::Loading => {
                    let msg = theme::style_dim("[ ⏳ Fetching synced lyrics... ]");
                    print!("\n\r\x1b[K  {}", msg);
                    print!("\n\r\x1b[K");
                    print!("\n\r\x1b[K");
                    lines_printed += 3;
                }
                LyricsState::Unavailable => {
                    let msg = theme::style_dim("[ 🎤 Synced lyrics unavailable ]");
                    print!("\n\r\x1b[K  {}", msg);
                    print!("\n\r\x1b[K");
                    print!("\n\r\x1b[K");
                    lines_printed += 3;
                }
                LyricsState::Loaded(lyrics_data) => {
                    let lines = &lyrics_data.lines;
                    let mut active_idx = None;
                    let mut is_instrumental = false;

                    for (i, l) in lines.iter().enumerate() {
                        let next_start = if i + 1 < lines.len() {
                            lines[i + 1].start_time
                        } else {
                            l.end_time.unwrap_or(l.start_time + Duration::from_secs(5))
                        };

                        if elapsed >= l.start_time && elapsed < l.end_time.unwrap_or(next_start) {
                            active_idx = Some(i);
                            break;
                        }

                        if i + 1 < lines.len() {
                            let next_line = &lines[i + 1];
                            if elapsed >= l.end_time.unwrap_or(l.start_time)
                                && elapsed < next_line.start_time
                            {
                                if next_line.start_time.saturating_sub(elapsed)
                                    <= Duration::from_millis(2500)
                                {
                                    active_idx = Some(i + 1);
                                } else {
                                    active_idx = Some(i);
                                    is_instrumental = true;
                                }
                                break;
                            }
                        }
                    }

                    if active_idx.is_none() {
                        if let Some(first) = lines.first() {
                            if elapsed < first.start_time {
                                if first.start_time.saturating_sub(elapsed)
                                    <= Duration::from_millis(2500)
                                {
                                    active_idx = Some(0);
                                }
                            } else if let Some(last) = lines.last() {
                                if elapsed >= last.start_time {
                                    active_idx = Some(lines.len() - 1);
                                }
                            }
                        }
                    }

                    if let Some(idx) = active_idx {
                        if is_instrumental {
                            let prev_str = theme::style_dim(&lines[idx].text).to_string();
                            let active_str = theme::style_dim("🎵 (Instrumental)").to_string();
                            let next_str = if idx + 1 < lines.len() {
                                theme::style_dim(&lines[idx + 1].text).to_string()
                            } else {
                                String::new()
                            };

                            print!("\n\r\x1b[K  {}", prev_str);
                            print!("\n\r\x1b[K  {}", active_str);
                            print!("\n\r\x1b[K  {}", next_str);
                            lines_printed += 3;
                        } else {
                            let prev_str = if idx > 0 {
                                theme::style_dim(&lines[idx - 1].text).to_string()
                            } else {
                                String::new()
                            };

                            let active_str =
                                crate::lyrics::render_active_line(&lines[idx], elapsed, visualizer);

                            let next_str = if idx + 1 < lines.len() {
                                theme::style_dim(&lines[idx + 1].text).to_string()
                            } else {
                                String::new()
                            };

                            print!("\n\r\x1b[K  {}", prev_str);
                            print!("\n\r\x1b[K  🎤 {}", active_str);
                            print!("\n\r\x1b[K  {}", next_str);
                            lines_printed += 3;
                        }
                    } else {
                        let first_str = if let Some(first) = lines.first() {
                            theme::style_dim(&first.text).to_string()
                        } else {
                            String::new()
                        };
                        print!("\n\r\x1b[K  ");
                        print!("\n\r\x1b[K  🎵 ...");
                        print!("\n\r\x1b[K  {}", first_str);
                        lines_printed += 3;
                    }
                }
            }
        }
    }

    if lines_printed > 0 {
        print!("\x1b[{}A\r", lines_printed);
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

fn print_album_table(albums: &[AlbumInfo]) {
    let mut table = theme::create_styled_table();
    table.set_header(vec![
        theme::style_header_cell("#"),
        theme::style_header_cell("Title"),
        theme::style_header_cell("Artist"),
        theme::style_header_cell("Year"),
    ]);

    for (i, album) in albums.iter().enumerate() {
        let year_str = match album.year {
            Some(y) => y.to_string(),
            None => "----".to_string(),
        };
        table.add_row(vec![
            theme::style_data_cell(&(i + 1).to_string(), false),
            theme::style_data_cell(&album.title, true),
            theme::style_data_cell(&album.artist, false),
            theme::style_data_cell(&year_str, false),
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
        seed = (a
            .wrapping_mul(seed)
            .wrapping_add(seed.rotate_left(11))
            .wrapping_add(c))
            % m;
        seed
    };

    let n = tracks.len();
    for i in (1..n).rev() {
        let j = (next_random() as usize) % (i + 1);
        tracks.swap(i, j);
    }
}

fn get_user_agent_for_gvs_url(stream_url: &str) -> &'static str {
    let val = if let Some(pos) = stream_url.find("?c=") {
        &stream_url[pos + 3..]
    } else if let Some(pos) = stream_url.find("&c=") {
        &stream_url[pos + 3..]
    } else {
        ""
    };

    let client = if let Some(end_pos) = val.find('&') {
        &val[..end_pos]
    } else {
        val
    };

    let client_upper = client.to_uppercase();
    if client_upper.contains("TV") {
        "Mozilla/5.0 (Chromecast; GoogleTV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/100.0.4896.127 Safari/537.36"
    } else if client_upper.contains("ANDROID") {
        "com.google.android.youtube/19.17.34 (Linux; U; Android 14; US) GMT+00:00"
    } else if client_upper.contains("IOS") {
        "com.google.ios.youtube/19.17.34 (iPhone16,2; U; CPU iPhone OS 17_5 like Mac OS X; US)"
    } else {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
    }
}

async fn play_progressive_track(
    video_id: &str,
    title: &str,
    artist: &str,
    config: &Config,
    queue_info: Option<(usize, usize)>,
    player: &AudioPlayer,
    current_volume: &f32,
    debug: bool,
    cache_file: &std::path::PathBuf,
) -> Result<(
    rodio::Sink,
    Arc<VisualizerShared>,
    Option<(
        tokio::task::JoinHandle<()>,
        std::path::PathBuf,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
)> {
    // Remove any partial/corrupted cache file on disk
    if cache_file.exists() {
        std::fs::remove_file(cache_file).ok();
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
            println!(
                "  ⏳ [{}/{}] Buffering: {}",
                idx + 1,
                total,
                theme::style_primary(&track_name)
            );
        } else {
            println!("  ⏳ Buffering: {}", theme::style_primary(&track_name));
        }
        None
    };

    let client = NetworkClient::new();
    let yt_dlp_path = config.ensure_yt_dlp().await?;
    let js_runtime = config.get_js_runtime_arg();
    let cookies_path = Some(config.cookies_path.as_path());
    let browser = config.get_cookie_browser_arg();

    // Extract the direct stream URL
    let stream_url = match client
        .get_stream_url(
            video_id,
            &yt_dlp_path,
            &js_runtime,
            cookies_path,
            browser.as_deref(),
        )
        .await
    {
        Ok(url) => url,
        Err(e) => {
            if let Some(ref spinner) = pb {
                spinner.finish_and_clear();
            }
            println!(
                "  ❌ Failed to get stream URL: {}",
                theme::style_error(&e.to_string())
            );
            return Err(e);
        }
    };

    // Create references for background task
    let cache_file_clone = cache_file.clone();
    let stream_url_clone = stream_url.clone();
    let download_complete = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let download_complete_clone = Arc::clone(&download_complete);
    let download_active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let download_active_clone = Arc::clone(&download_active);
    let total_size = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_size_clone = Arc::clone(&total_size);

    // Spawn a background task to download the stream using reqwest
    let download_handle = tokio::spawn(async move {
        let mut success = false;

        let res = async {
            let ua = get_user_agent_for_gvs_url(&stream_url_clone);
            let req_client = reqwest::Client::builder()
                .user_agent(ua)
                .build()
                .map_err(|e| {
                    log::error!("Progressive download failed to build reqwest client: {}", e);
                })?;

            let mut response = req_client
                .get(&stream_url_clone)
                .send()
                .await
                .map_err(|e| {
                    log::error!("Progressive download failed to send request: {}", e);
                })?;

            // Read Content-Length
            if let Some(content_len) = response.content_length() {
                total_size_clone.store(content_len, std::sync::atomic::Ordering::SeqCst);
            }

            let mut file = std::fs::File::create(&cache_file_clone).map_err(|e| {
                log::error!("Progressive download failed to create file: {}", e);
            })?;

            // Buffer stream bytes
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        use std::io::Write;
                        if let Err(e) = file.write_all(&chunk) {
                            log::error!("Progressive download failed to write to file: {}", e);
                            break;
                        }
                    }
                    Ok(None) => {
                        success = true;
                        break;
                    }
                    Err(e) => {
                        log::error!("Progressive download network error: {}", e);
                        break;
                    }
                }
            }
            Ok::<(), ()>(())
        }
        .await;

        if res.is_ok() && success {
            download_complete_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        download_active_clone.store(false, std::sync::atomic::Ordering::SeqCst);
    });

    // Wait a small amount of time for initial bytes to buffer so Symphonia can probe it
    let mut attempts = 0;
    loop {
        if download_complete.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if !download_active.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if cache_file.exists() {
            if let Ok(meta) = std::fs::metadata(cache_file) {
                // We need at least 16KB for the container headers to be fully read
                if meta.len() > 16384 {
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        attempts += 1;
        if attempts > 50 {
            // 5 seconds timeout
            break;
        }
    }

    if let Some(ref spinner) = pb {
        spinner.finish_and_clear();
    }

    // Verify that the file was created and contains data
    let file_ok = cache_file.exists()
        && std::fs::metadata(cache_file)
            .map(|m| {
                m.len() >= 16384 || download_complete.load(std::sync::atomic::Ordering::SeqCst)
            })
            .unwrap_or(false);
    if !file_ok {
        if cache_file.exists() {
            std::fs::remove_file(cache_file).ok();
        }
        return Err(anyhow::anyhow!(
            "Progressive download failed to initialize stream (download failed or incomplete)"
        ));
    }

    // Open the file for progressive playback
    let file = std::fs::File::open(cache_file)?;
    let ext = cache_file.extension().and_then(|e| e.to_str());
    let (sink, _total_dur, vis) = match player.play_progressive(
        file,
        ext,
        Arc::clone(&download_complete),
        Arc::clone(&download_active),
        Arc::clone(&total_size),
    ) {
        Ok(res) => res,
        Err(e) => {
            if cache_file.exists() {
                std::fs::remove_file(cache_file).ok();
            }
            return Err(e);
        }
    };
    sink.set_volume(*current_volume);

    Ok((
        sink,
        vis,
        Some((download_handle, cache_file.clone(), download_complete)),
    ))
}

fn is_matching_alternative(original: &TrackInfo, candidate: &TrackInfo) -> bool {
    let orig_title = original.title.to_lowercase();
    let cand_title = candidate.title.to_lowercase();
    let orig_artist = original.artist.to_lowercase();
    let cand_artist = candidate.artist.to_lowercase();

    // 1. Artist Match Check
    let artist_match = cand_artist.contains(&orig_artist)
        || orig_artist.contains(&cand_artist)
        || cand_artist.split(&[' ', ',', '&', '-'][..]).any(|part| {
            let p = part.trim();
            p.len() > 2 && orig_artist.contains(p)
        });

    if !artist_match {
        return false;
    }

    // 2. Duration Match Check
    if let (Some(orig_dur), Some(cand_dur)) = (original.duration_secs, candidate.duration_secs) {
        let diff = (orig_dur as i64 - cand_dur as i64).abs();
        if diff > 45 {
            return false;
        }
    }

    // 3. Title Match Check
    fn clean_title(title: &str) -> String {
        let mut cleaned = title.to_string();
        let suffixes = [
            "(official video)",
            "(official audio)",
            "(audio)",
            "(video)",
            "[lyric video]",
            "(lyric video)",
            "(lyrics)",
            "[lyrics]",
            "(official lyric video)",
            "(official music video)",
            "[music video]",
            "(hq)",
            "(hd)",
        ];
        for suffix in suffixes {
            cleaned = cleaned.replace(suffix, "");
        }
        cleaned.trim().to_string()
    }

    let clean_orig = clean_title(&orig_title);
    let clean_cand = clean_title(&cand_title);

    use fuzzy_matcher::FuzzyMatcher;
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    if matcher.fuzzy_match(&clean_cand, &clean_orig).is_some() {
        let orig_words: Vec<&str> = clean_orig
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .collect();
        if orig_words.is_empty() {
            return true;
        }
        let matched_words = orig_words
            .iter()
            .filter(|&&w| clean_cand.contains(w))
            .count();
        let containment_ok = matched_words as f32 / orig_words.len() as f32 >= 0.6;
        containment_ok
    } else {
        false
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

    if let Some(discord) = config.get_discord_settings() {
        if discord.enabled {
            clear_screen();
            let track_name = format!("{} - {}", title, artist);
            if let Some((idx, total)) = queue_info {
                println!(
                    "  💿 [{}/{}] Playing (Discord Mode): {}",
                    idx + 1,
                    total,
                    theme::style_primary(&track_name)
                );
            } else {
                println!(
                    "  💿 Playing (Discord Mode): {}",
                    theme::style_primary(&track_name)
                );
            }

            let controls_help = if queue_info.is_some() {
                "  🎮 Controls: [Space] Play/Pause  [N] Skip  [P] Prev  [Q] Stop/Back"
            } else {
                "  🎮 Controls: [Space] Play/Pause  [Q] Stop/Back"
            };
            println!("{}", theme::style_dim(controls_help));

            let cmd_text = format!("m!play https://www.youtube.com/watch?v={}", video_id);
            if let Err(e) =
                crate::discord::send_discord_command(&discord.token, &discord.channel_id, &cmd_text)
                    .await
            {
                println!(
                    "  ❌ Failed to send command to Discord: {}",
                    theme::style_error(&e.to_string())
                );
                press_enter_to_continue();
                return Ok(PlaybackControl::Quit);
            }
            db.add_history(video_id, title, artist)?;

            let total_dur = total_duration.unwrap_or(Duration::from_secs(180));
            let mut elapsed = Duration::from_secs(0);
            let mut last_tick = Instant::now();
            let mut is_paused = false;
            let mut control = PlaybackControl::Finished;

            {
                let _guard = RawModeGuard::new()?;
                while elapsed < total_dur {
                    let now = Instant::now();
                    let delta = now.duration_since(last_tick);
                    last_tick = now;

                    if !is_paused {
                        elapsed += delta;
                    }

                    draw_progress_bar(elapsed, total_duration, 1.0, is_paused, None, None, false);

                    if event::poll(Duration::from_millis(100))? {
                        if let Event::Key(key_event) = event::read()? {
                            if key_event.kind == event::KeyEventKind::Press
                                || key_event.kind == event::KeyEventKind::Repeat
                            {
                                match key_event.code {
                                    KeyCode::Char(' ') => {
                                        is_paused = !is_paused;
                                        let cmd = if is_paused { "m!pause" } else { "m!resume" };
                                        crate::discord::send_discord_command(
                                            &discord.token,
                                            &discord.channel_id,
                                            cmd,
                                        )
                                        .await
                                        .ok();
                                    }
                                    KeyCode::Char('n') | KeyCode::Char('N') => {
                                        if queue_info.is_some() {
                                            crate::discord::send_discord_command(
                                                &discord.token,
                                                &discord.channel_id,
                                                "m!skip",
                                            )
                                            .await
                                            .ok();
                                            control = PlaybackControl::Next;
                                            break;
                                        }
                                    }
                                    KeyCode::Char('p') | KeyCode::Char('P') => {
                                        if queue_info.is_some() {
                                            crate::discord::send_discord_command(
                                                &discord.token,
                                                &discord.channel_id,
                                                "m!skip",
                                            )
                                            .await
                                            .ok();
                                            control = PlaybackControl::Prev;
                                            break;
                                        }
                                    }
                                    KeyCode::Char('q') | KeyCode::Esc => {
                                        crate::discord::send_discord_command(
                                            &discord.token,
                                            &discord.channel_id,
                                            "m!stop",
                                        )
                                        .await
                                        .ok();
                                        control = PlaybackControl::Quit;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                print!("\r\x1b[K");
                std::io::stdout().flush().ok();
            }

            return Ok(control);
        }
    }

    let m4a_file = config.cache_dir.join(format!("{}.m4a", video_id));

    let cached = db.get_cached_track(video_id)?;
    let (use_cache, cache_file) = if let Some(ref c) = cached {
        let path = std::path::Path::new(&c.file_path);
        if path.exists() {
            (true, path.to_path_buf())
        } else {
            (false, m4a_file)
        }
    } else {
        (false, m4a_file)
    };

    let player = AudioPlayer::new()?;

    let client_id = config
        .get_discord_settings()
        .map(|s| s.client_id)
        .unwrap_or_else(|| "1089228496459345970".to_string());

    let mut rpc = crate::discord_rpc::DiscordRpc::new(config);
    match rpc.connect(&client_id) {
        Ok(_) => {
            if debug {
                println!(
                    "  🤖 {}",
                    theme::style_primary("Successfully connected to Discord RPC")
                );
                std::io::stdout().flush().ok();
            }
            rpc.update(track, Duration::from_secs(0), false);
        }
        Err(e) => {
            if debug {
                println!(
                    "  🤖 {}",
                    theme::style_error(&format!("Discord RPC connection failed: {}", e))
                );
                std::io::stdout().flush().ok();
            }
        }
    }

    // Keep active playback details fresh on a cleared screen
    clear_screen();

    let res = if use_cache {
        let track_name = format!("{} - {}", title, artist);
        db.update_cached_track_accessed(video_id).ok();
        match player.play_local(cache_file.clone()) {
            Ok((sink, _total_dur, vis)) => {
                if let Some((idx, total)) = queue_info {
                    println!(
                        "  💿 [{}/{}] Playing (cached): {}",
                        idx + 1,
                        total,
                        theme::style_primary(&track_name)
                    );
                } else {
                    println!(
                        "  💿 Playing (cached): {}",
                        theme::style_primary(&track_name)
                    );
                }
                sink.set_volume(*current_volume);
                Ok((sink, vis, None))
            }
            Err(e) => {
                log::warn!(
                    "Cache file corrupted for {}: {}. Deleting cache and streaming...",
                    video_id,
                    e
                );
                std::fs::remove_file(&cache_file).ok();
                db.delete_cached_track(video_id).ok();
                let progressive_cache_file = config.cache_dir.join(format!("{}.m4a", video_id));
                play_progressive_track(
                    video_id,
                    title,
                    artist,
                    config,
                    queue_info,
                    &player,
                    current_volume,
                    debug,
                    &progressive_cache_file,
                )
                .await
            }
        }
    } else {
        play_progressive_track(
            video_id,
            title,
            artist,
            config,
            queue_info,
            &player,
            current_volume,
            debug,
            &cache_file,
        )
        .await
    };

    let (sink, visualizer_shared, download_info) = match res {
        Ok(vals) => vals,
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("This video is only available to Music Premium members") {
                println!(
                    "  🔍 '{}' is premium-only. Searching for a public version...",
                    theme::style_primary(title)
                );
                std::io::stdout().flush().ok();
                let search_query = format!("{} {}", title, artist);
                let client = NetworkClient::new();
                match client.search(&search_query).await {
                    Ok(results) => {
                        let alternative = results
                            .into_iter()
                            .find(|t| t.id != track.id && is_matching_alternative(track, t));
                        if let Some(alt_track) = alternative {
                            println!(
                                "  🔄 Found alternative: '{}' by '{}'. Playing alternative...",
                                theme::style_primary(&alt_track.title),
                                theme::style_primary(&alt_track.artist)
                            );
                            std::io::stdout().flush().ok();
                            let alt_cache_file =
                                config.cache_dir.join(format!("{}.m4a", alt_track.id));
                            play_progressive_track(
                                &alt_track.id,
                                &alt_track.title,
                                &alt_track.artist,
                                config,
                                queue_info,
                                &player,
                                current_volume,
                                debug,
                                &alt_cache_file,
                            )
                            .await?
                        } else {
                            return Err(e);
                        }
                    }
                    Err(_) => return Err(e),
                }
            } else {
                return Err(e);
            }
        }
    };

    db.add_history(video_id, title, artist)?;

    let lyrics_enabled = db
        .get_setting("lyrics_enabled")
        .ok()
        .flatten()
        .unwrap_or_else(|| "true".to_string())
        == "true";

    let (lyrics_state, lyrics_task_handle) = if lyrics_enabled {
        let state = Arc::new(std::sync::RwLock::new(LyricsState::Loading));
        let state_clone = Arc::clone(&state);
        let title_clone = title.to_string();
        let artist_clone = artist.to_string();
        let duration_secs = track.duration_secs;

        let handle = tokio::spawn(async move {
            match crate::lyrics::fetch_lyrics(&title_clone, &artist_clone, duration_secs).await {
                Ok(Some(data)) => {
                    if let Ok(mut w) = state_clone.write() {
                        *w = LyricsState::Loaded(data);
                    }
                }
                _ => {
                    if let Ok(mut w) = state_clone.write() {
                        *w = LyricsState::Unavailable;
                    }
                }
            }
        });
        (Some(state), Some(handle))
    } else {
        (None, None)
    };

    let mut show_lyrics = lyrics_enabled;

    let controls_help = match (queue_info.is_some(), lyrics_enabled) {
        (true, true) => {
            "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [L] Lyrics  [N] Next  [P] Prev  [Q] Stop/Back"
        }
        (true, false) => {
            "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [N] Next  [P] Prev  [Q] Stop/Back"
        }
        (false, true) => {
            "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [L] Lyrics  [Q] Stop/Back"
        }
        (false, false) => {
            "  🎮 Controls: [Space] Play/Pause  [←/→] Seek  [↑/↓] Volume  [Q] Stop/Back"
        }
    };
    println!("{}", theme::style_dim(controls_help));

    let mut control = PlaybackControl::Finished;

    // Scope for raw mode guard
    {
        let _guard = RawModeGuard::new()?;
        let mut last_seek = Instant::now() - Duration::from_secs(1);

        while !sink.empty() {
            let elapsed = Duration::from_millis(visualizer_shared.get_elapsed_ms());

            let l_state = lyrics_state.as_ref().and_then(|s| s.read().ok());
            draw_progress_bar(
                elapsed,
                total_duration,
                sink.volume(),
                sink.is_paused(),
                Some(&visualizer_shared),
                l_state.as_deref(),
                show_lyrics,
            );

            if event::poll(Duration::from_millis(40))? {
                if let Event::Key(key_event) = event::read()? {
                    if key_event.kind == event::KeyEventKind::Press
                        || key_event.kind == event::KeyEventKind::Repeat
                    {
                        match key_event.code {
                            KeyCode::Char(' ') => {
                                if sink.is_paused() {
                                    sink.play();
                                    rpc.update(track, elapsed, false);
                                } else {
                                    sink.pause();
                                    rpc.update(track, elapsed, true);
                                }
                            }
                            KeyCode::Char('l') | KeyCode::Char('L') => {
                                show_lyrics = !show_lyrics;
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
                                if now_seek.duration_since(last_seek) >= Duration::from_millis(250)
                                {
                                    let new_pos = elapsed.saturating_sub(Duration::from_secs(5));
                                    match sink.try_seek(new_pos) {
                                        Ok(_) => {
                                            visualizer_shared
                                                .set_elapsed_ms(new_pos.as_millis() as u64);
                                            last_seek = now_seek;
                                            rpc.update(track, new_pos, sink.is_paused());
                                        }
                                        Err(e) => {
                                            print!("\r\n  ❌ Left seek failed: {:?}\r\n", e);
                                            std::io::stdout().flush().ok();
                                        }
                                    }
                                }
                            }
                            KeyCode::Right => {
                                let now_seek = Instant::now();
                                if now_seek.duration_since(last_seek) >= Duration::from_millis(250)
                                {
                                    let new_pos = elapsed + Duration::from_secs(5);
                                    let can_seek = match total_duration {
                                        Some(total_dur) => new_pos < total_dur,
                                        None => true,
                                    };
                                    if can_seek {
                                        match sink.try_seek(new_pos) {
                                            Ok(_) => {
                                                visualizer_shared
                                                    .set_elapsed_ms(new_pos.as_millis() as u64);
                                                last_seek = now_seek;
                                                rpc.update(track, new_pos, sink.is_paused());
                                            }
                                            Err(e) => {
                                                print!("\r\n  ❌ Right seek failed: {:?}\r\n", e);
                                                std::io::stdout().flush().ok();
                                            }
                                        }
                                    }
                                }
                            }
                            KeyCode::Up => {
                                let vol = (sink.volume() + 0.01).min(2.0);
                                sink.set_volume(vol);
                                *current_volume = vol;
                            }
                            KeyCode::Down => {
                                let vol = (sink.volume() - 0.01).max(0.0);
                                sink.set_volume(vol);
                                *current_volume = vol;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        // Abort background lyrics task if still running
        if let Some(h) = lyrics_task_handle {
            h.abort();
        }

        // Clear all rendered UI lines (visualizer + progress bar + optional lyrics section)
        let total_lines_to_clear = if show_lyrics && lyrics_state.is_some() {
            12
        } else {
            8
        };
        for _ in 0..total_lines_to_clear {
            print!("\n\r\x1b[K");
        }
        print!("\x1b[{}A\r\x1b[K", total_lines_to_clear);
        std::io::stdout().flush().ok();
        rpc.clear();
    }

    // Clean up / Register progressive download
    if let Some((handle, path, complete)) = download_info {
        if !complete.load(std::sync::atomic::Ordering::SeqCst) {
            handle.abort();
            let path_clone = path.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                std::fs::remove_file(path_clone).ok();
            });
        } else {
            let duration_u32 = total_duration.map(|d| d.as_secs() as u32).unwrap_or(0);
            let file_path_str = path.to_string_lossy().to_string();
            if let Err(e) =
                db.insert_cached_track(video_id, title, artist, duration_u32, &file_path_str)
            {
                eprintln!("  Failed to register cached track in DB: {}", e);
            }
            let max_bytes = db.get_max_cache_size_bytes();
            let _ = db.enforce_cache_limit(&config.cache_dir, max_bytes);
        }
    }

    Ok(control)
}

fn spawn_prefetch(track: TrackInfo, config: Config, db: Db) -> tokio::task::JoinHandle<()> {
    let lyrics_enabled = db
        .get_setting("lyrics_enabled")
        .ok()
        .flatten()
        .unwrap_or_else(|| "true".to_string())
        == "true";

    if lyrics_enabled {
        let title_clone = track.title.clone();
        let artist_clone = track.artist.clone();
        let duration_secs = track.duration_secs;
        tokio::spawn(async move {
            let _ = crate::lyrics::fetch_lyrics(&title_clone, &artist_clone, duration_secs).await;
        });
    }

    tokio::spawn(async move {
        let video_id = &track.id;
        let flac_file = config.cache_dir.join(format!("{}.flac", video_id));
        let m4a_file = config.cache_dir.join(format!("{}.m4a", video_id));

        // Check if already cached and registered in DB
        let is_cached = if let Ok(Some(_)) = db.get_cached_track(video_id) {
            flac_file.exists() || m4a_file.exists()
        } else {
            false
        };

        if is_cached {
            return;
        }

        // Clean up any partial/orphaned files on disk from prior aborted tasks
        if flac_file.exists() {
            std::fs::remove_file(&flac_file).ok();
        }
        if m4a_file.exists() {
            std::fs::remove_file(&m4a_file).ok();
        }

        let cache_file = m4a_file;

        if let Ok(yt_dlp_path) = config.ensure_yt_dlp().await {
            let client = NetworkClient::new();
            let js_runtime = config.get_js_runtime_arg();
            let cookies_path = Some(config.cookies_path.as_path());
            let browser = config.get_cookie_browser_arg();

            // Extract the stream URL
            let stream_url = match client
                .get_stream_url(
                    video_id,
                    &yt_dlp_path,
                    &js_runtime,
                    cookies_path,
                    browser.as_deref(),
                )
                .await
            {
                Ok(url) => url,
                Err(e) => {
                    log::error!("Prefetch failed to get stream URL: {}", e);
                    return;
                }
            };

            let ua = get_user_agent_for_gvs_url(&stream_url);
            let req_client = match reqwest::Client::builder().user_agent(ua).build() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Prefetch failed to build reqwest client: {}", e);
                    return;
                }
            };

            let mut response = match req_client.get(&stream_url).send().await {
                Ok(res) => res,
                Err(e) => {
                    log::error!("Prefetch failed to send request: {}", e);
                    return;
                }
            };

            if !response.status().is_success() {
                log::error!(
                    "Prefetch download returned error status: {}",
                    response.status()
                );
                return;
            }

            let mut file = match std::fs::File::create(&cache_file) {
                Ok(f) => f,
                Err(e) => {
                    log::error!("Prefetch failed to create file: {}", e);
                    return;
                }
            };

            let mut success = false;
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        use std::io::Write;
                        if let Err(e) = file.write_all(&chunk) {
                            log::error!("Prefetch failed to write to file: {}", e);
                            break;
                        }
                    }
                    Ok(None) => {
                        success = true;
                        break;
                    }
                    Err(e) => {
                        log::error!("Prefetch network error: {}", e);
                        break;
                    }
                }
            }

            if !success {
                std::fs::remove_file(&cache_file).ok();
                return;
            }

            let duration_u32 = track.duration_secs.unwrap_or(0);
            let file_path_str = cache_file.to_string_lossy().to_string();
            let _ = db.insert_cached_track(
                video_id,
                &track.title,
                &track.artist,
                duration_u32,
                &file_path_str,
            );
            let max_bytes = db.get_max_cache_size_bytes();
            let _ = db.enforce_cache_limit(&config.cache_dir, max_bytes);
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
    if tracks.is_empty() {
        return Ok(());
    }
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
        let control_res = play_track(
            track,
            config,
            db,
            Some((idx, tracks.len())),
            current_volume,
            debug,
        )
        .await;

        match control_res {
            Ok(control) => match control {
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
            },
            Err(e) => {
                println!(
                    "\n  ❌ Playback failed for '{}': {}",
                    theme::style_primary(&track.title),
                    theme::style_error(&e.to_string())
                );
                std::io::stdout().flush().ok();
                tokio::time::sleep(Duration::from_secs(2)).await;
                idx += 1;
                if idx >= tracks.len() {
                    println!("\n  Queue finished.");
                    press_enter_to_continue();
                    break;
                }
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
        table.set_header(
            headers
                .iter()
                .map(|h| theme::style_header_cell(h))
                .collect::<Vec<_>>(),
        );

        for (i, track) in tracks.iter().enumerate() {
            let is_selected = i == selected_idx;
            let dur_str = match track.duration_secs {
                Some(d) => format_duration(Duration::from_secs(d as u64)),
                None => "--:--".to_string(),
            };

            let num_cell = if is_selected {
                comfy_table::Cell::new(format!("> {}", i + 1))
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(format!("  {}", i + 1)).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let title_cell = if is_selected {
                comfy_table::Cell::new(&track.title)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_ACCENT.0,
                        g: theme::COLOR_ACCENT.1,
                        b: theme::COLOR_ACCENT.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&track.title).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let artist_cell = if is_selected {
                comfy_table::Cell::new(&track.artist)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&track.artist).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let dur_cell = if is_selected {
                comfy_table::Cell::new(dur_str)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(dur_str).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            table.add_row(vec![num_cell, title_cell, artist_cell, dur_cell]);
        }

        // Add Go Back row
        let is_selected = selected_idx == tracks.len();
        let num_cell = if is_selected {
            comfy_table::Cell::new("> 🔙").add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new("  🔙")
        };
        let label_cell = if is_selected {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_PRIMARY.0,
                    g: theme::COLOR_PRIMARY.1,
                    b: theme::COLOR_PRIMARY.2,
                })
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new(go_back_label).fg(comfy_table::Color::Rgb {
                r: theme::COLOR_DIM.0,
                g: theme::COLOR_DIM.1,
                b: theme::COLOR_DIM.2,
            })
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

        print!(
            "\r\n  {}\r\n",
            theme::style_dim("🎮 Controls: [↑/↓] Move  [Enter] Select  [Q/Esc] Back")
        );
        std::io::stdout().flush().ok();

        if let Event::Key(key_event) = event::read()? {
            if key_event.kind == event::KeyEventKind::Press
                || key_event.kind == event::KeyEventKind::Repeat
            {
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

fn interact_album_table_select(
    prompt: &str,
    headers: Vec<&str>,
    albums: &[AlbumInfo],
    go_back_label: &str,
) -> Result<Option<usize>> {
    let mut selected_idx = 0;
    let num_options = albums.len() + 1;

    let _guard = RawModeGuard::new()?;

    loop {
        clear_screen();
        print!("  ❓ {}\r\n\r\n", theme::style_primary(prompt));

        let mut table = theme::create_styled_table();
        table.set_header(
            headers
                .iter()
                .map(|h| theme::style_header_cell(h))
                .collect::<Vec<_>>(),
        );

        for (i, album) in albums.iter().enumerate() {
            let is_selected = i == selected_idx;
            let year_str = match album.year {
                Some(y) => y.to_string(),
                None => "----".to_string(),
            };

            let num_cell = if is_selected {
                comfy_table::Cell::new(format!("> {}", i + 1))
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(format!("  {}", i + 1)).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let title_cell = if is_selected {
                comfy_table::Cell::new(&album.title)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_ACCENT.0,
                        g: theme::COLOR_ACCENT.1,
                        b: theme::COLOR_ACCENT.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&album.title).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let artist_cell = if is_selected {
                comfy_table::Cell::new(&album.artist)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&album.artist).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let year_cell = if is_selected {
                comfy_table::Cell::new(year_str)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(year_str).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            table.add_row(vec![num_cell, title_cell, artist_cell, year_cell]);
        }

        // Add Go Back row
        let is_selected = selected_idx == albums.len();
        let num_cell = if is_selected {
            comfy_table::Cell::new("> 🔙").add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new("  🔙")
        };
        let label_cell = if is_selected {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_PRIMARY.0,
                    g: theme::COLOR_PRIMARY.1,
                    b: theme::COLOR_PRIMARY.2,
                })
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new(go_back_label).fg(comfy_table::Color::Rgb {
                r: theme::COLOR_DIM.0,
                g: theme::COLOR_DIM.1,
                b: theme::COLOR_DIM.2,
            })
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

        print!(
            "\r\n  {}\r\n",
            theme::style_dim("🎮 Controls: [↑/↓] Move  [Enter] Select  [Q/Esc] Back")
        );
        std::io::stdout().flush().ok();

        if let Event::Key(key_event) = event::read()? {
            if key_event.kind == event::KeyEventKind::Press
                || key_event.kind == event::KeyEventKind::Repeat
            {
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('p') | KeyCode::Char('P') => {
                        selected_idx = (selected_idx + num_options - 1) % num_options;
                    }
                    KeyCode::Down | KeyCode::Char('n') | KeyCode::Char('N') => {
                        selected_idx = (selected_idx + 1) % num_options;
                    }
                    KeyCode::Enter => {
                        if selected_idx == albums.len() {
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
        table.set_header(
            headers
                .iter()
                .map(|h| theme::style_header_cell(h))
                .collect::<Vec<_>>(),
        );

        for (i, entry) in history.iter().enumerate() {
            let is_selected = i == selected_idx;
            let short_date = if entry.played_at.len() > 19 {
                &entry.played_at[..19]
            } else {
                &entry.played_at
            };

            let num_cell = if is_selected {
                comfy_table::Cell::new(format!("> {}", i + 1))
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(format!("  {}", i + 1)).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let title_cell = if is_selected {
                comfy_table::Cell::new(&entry.title)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_ACCENT.0,
                        g: theme::COLOR_ACCENT.1,
                        b: theme::COLOR_ACCENT.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&entry.title).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let artist_cell = if is_selected {
                comfy_table::Cell::new(&entry.artist)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(&entry.artist).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            let date_cell = if is_selected {
                comfy_table::Cell::new(short_date)
                    .fg(comfy_table::Color::Rgb {
                        r: theme::COLOR_PRIMARY.0,
                        g: theme::COLOR_PRIMARY.1,
                        b: theme::COLOR_PRIMARY.2,
                    })
                    .add_attribute(comfy_table::Attribute::Bold)
            } else {
                comfy_table::Cell::new(short_date).fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_DIM.0,
                    g: theme::COLOR_DIM.1,
                    b: theme::COLOR_DIM.2,
                })
            };

            table.add_row(vec![num_cell, title_cell, artist_cell, date_cell]);
        }

        // Add Go Back row
        let is_selected = selected_idx == history.len();
        let num_cell = if is_selected {
            comfy_table::Cell::new("> 🔙").add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new("  🔙")
        };
        let label_cell = if is_selected {
            comfy_table::Cell::new(go_back_label)
                .fg(comfy_table::Color::Rgb {
                    r: theme::COLOR_PRIMARY.0,
                    g: theme::COLOR_PRIMARY.1,
                    b: theme::COLOR_PRIMARY.2,
                })
                .add_attribute(comfy_table::Attribute::Bold)
        } else {
            comfy_table::Cell::new(go_back_label).fg(comfy_table::Color::Rgb {
                r: theme::COLOR_DIM.0,
                g: theme::COLOR_DIM.1,
                b: theme::COLOR_DIM.2,
            })
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

        print!(
            "\r\n  {}\r\n",
            theme::style_dim("🎮 Controls: [↑/↓] Move  [Enter] Select  [Q/Esc] Back")
        );
        std::io::stdout().flush().ok();

        if let Event::Key(key_event) = event::read()? {
            if key_event.kind == event::KeyEventKind::Press
                || key_event.kind == event::KeyEventKind::Repeat
            {
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

async fn run_search_and_play(
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    loop {
        clear_screen();
        let query: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Search YouTube Music (or 'q' to go back)")
            .allow_empty(true)
            .interact_text()?;

        let query = query.trim();
        if query.is_empty() {
            continue;
        }
        if query.eq_ignore_ascii_case("q") {
            break;
        }

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
            let mut queue = vec![selected_track.clone()];
            let mut autoplay_ctoken = None;

            let discord_active = config
                .get_discord_settings()
                .map(|d| d.enabled)
                .unwrap_or(false);
            if !discord_active {
                println!("\n  📻 Fetching autoplay recommendations...");
                match client.fetch_autoplay_queue(&selected_track.id).await {
                    Ok((mut autoplay_tracks, ctoken)) => {
                        queue.append(&mut autoplay_tracks);
                        autoplay_ctoken = ctoken;
                    }
                    Err(e) => {
                        log::warn!("Failed to fetch autoplay tracks: {}", e);
                    }
                }
            }

            play_queue(
                queue,
                0,
                config,
                db,
                client,
                autoplay_ctoken,
                current_volume,
                debug,
            )
            .await?;
        }
    }
    Ok(())
}

async fn run_search_albums_and_play(
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    loop {
        clear_screen();
        let query: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Search Albums on YouTube Music (or 'q' to go back)")
            .allow_empty(true)
            .interact_text()?;

        let query = query.trim();
        if query.is_empty() {
            continue;
        }
        if query.eq_ignore_ascii_case("q") {
            break;
        }

        println!(
            "\n  🔍 Searching for albums matching '{}'...",
            theme::style_primary(query)
        );
        let albums = client.search_albums(query).await?;

        if albums.is_empty() {
            println!("  ❌ {}", theme::style_error("No albums found."));
            press_enter_to_continue();
            continue;
        }

        loop {
            let selection = match interact_album_table_select(
                "Select album to play",
                vec!["#", "Title", "Artist", "Year"],
                &albums,
                "Search again / Go back",
            )? {
                Some(idx) => idx,
                None => break,
            };

            let selected_album = &albums[selection];
            println!(
                "\n  💿 Loading tracks for album '{}'...",
                theme::style_primary(&selected_album.title)
            );

            let album_url = format!(
                "https://music.youtube.com/playlist?list={}",
                selected_album.id
            );

            match fetch_playlist_with_retry(config, client, &album_url).await {
                Ok(tracks) => {
                    if tracks.is_empty() {
                        println!("  ❌ {}", theme::style_error("This album has no tracks."));
                        press_enter_to_continue();
                    } else {
                        play_queue(tracks, 0, config, db, client, None, current_volume, debug)
                            .await?;
                    }
                }
                Err(e) => {
                    println!(
                        "  ❌ {} {}",
                        theme::style_error("Failed to load album tracks:"),
                        e
                    );
                    press_enter_to_continue();
                }
            }
        }
    }
    Ok(())
}

async fn run_history(
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
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
        let mut queue = vec![track.clone()];
        let mut autoplay_ctoken = None;

        let discord_active = config
            .get_discord_settings()
            .map(|d| d.enabled)
            .unwrap_or(false);
        if !discord_active {
            println!("\n  📻 Fetching autoplay recommendations...");
            match client.fetch_autoplay_queue(&track.id).await {
                Ok((mut autoplay_tracks, ctoken)) => {
                    queue.append(&mut autoplay_tracks);
                    autoplay_ctoken = ctoken;
                }
                Err(e) => {
                    log::warn!("Failed to fetch autoplay tracks: {}", e);
                }
            }
        }
        play_queue(
            queue,
            0,
            config,
            db,
            client,
            autoplay_ctoken,
            current_volume,
            debug,
        )
        .await?;
    }
    Ok(())
}

async fn run_cache_menu(config: &Config, db: &Db) -> Result<()> {
    loop {
        clear_screen();
        let selections = &[
            "💾 List cached tracks",
            "⚙️ Configure cache size limit",
            "🗑️ Clear cache",
            "🔙 Go back",
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
                        let dur_str =
                            format_duration(Duration::from_secs(track.duration_secs as u64));
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
                let current_limit = db
                    .get_setting("max_cache_size_mb")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "50".to_string());
                println!(
                    "  Current cache size limit: {} MB",
                    theme::style_primary(&current_limit)
                );

                let input: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt("Enter new cache size limit in MB")
                    .default(current_limit)
                    .interact_text()?;

                if let Ok(mb) = input.trim().parse::<u64>() {
                    if mb > 0 {
                        db.set_setting("max_cache_size_mb", &mb.to_string())?;
                        println!(
                            "  ✨ Cache size limit updated to {} MB.",
                            theme::style_primary(&mb.to_string())
                        );
                        let _ = db.enforce_cache_limit(&config.cache_dir, mb * 1024 * 1024);
                    } else {
                        println!("  ❌ Limit must be greater than 0.");
                    }
                } else {
                    println!("  ❌ Invalid number.");
                }
                press_enter_to_continue();
            }
            2 => {
                let tracks = db.list_cached_tracks()?;
                let count = tracks.len();

                if count == 0 {
                    println!("  No cached tracks to clear.");
                    press_enter_to_continue();
                    continue;
                }

                let confirm = dialoguer::Confirm::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt(format!(
                        "Are you sure you want to delete {} cached track(s)?",
                        count
                    ))
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
                    println!(
                        "  ✨ {}",
                        theme::style_primary(&format!("Cleared {} cached track(s).", count))
                    );
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
        "zen",
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
    println!(
        "  Extracting cookies from {}...",
        theme::style_primary(browser)
    );
    match config.login(browser).await {
        Err(e) => {
            println!("  ❌ {}", theme::style_error(&e.to_string()));
        }
        _ => {
            println!("  ✨ {}", theme::style_primary("Logged in successfully!"));
        }
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
    let tracks = match fetch_playlist_with_retry(config, client, &url).await {
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
        println!(
            "\n  ── Playlist: {} ──",
            theme::style_primary(playlist_title)
        );
        print_track_table(&tracks);

        let mut options = vec!["▶️ Play All".to_string(), "🔀 Shuffle Play".to_string()];
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
            play_queue(
                tracks.clone(),
                0,
                config,
                db,
                client,
                None,
                current_volume,
                debug,
            )
            .await?;
        } else if selection == 1 {
            let mut shuffled_tracks = tracks.clone();
            shuffle_tracks(&mut shuffled_tracks);
            play_queue(
                shuffled_tracks,
                0,
                config,
                db,
                client,
                None,
                current_volume,
                debug,
            )
            .await?;
        } else if selection == options.len() - 1 {
            break;
        } else {
            let track_idx = selection - 2;
            play_queue(
                tracks.clone(),
                track_idx,
                config,
                db,
                client,
                None,
                current_volume,
                debug,
            )
            .await?;
        }
    }
    Ok(())
}

async fn run_library_playlists(
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    loop {
        clear_screen();
        let yt_dlp_path = config.ensure_yt_dlp().await?;

        println!("\n  Fetching your playlists...");
        let browser = config.get_cookie_browser_arg();
        let playlists = match client
            .fetch_library_playlists(
                &yt_dlp_path,
                browser.as_deref(),
                &config.cookies_path,
                &config.get_js_runtime_arg(),
            )
            .await
        {
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

        let mut options: Vec<String> = playlists
            .iter()
            .map(|p| {
                let count_str = match p.track_count {
                    Some(c) => format!(" ({} tracks)", c),
                    None => "".to_string(),
                };
                format!("📁 {}{}", p.title, count_str)
            })
            .collect();
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
        run_playlist_playback(
            &playlist.id,
            &playlist.title,
            config,
            db,
            client,
            current_volume,
            debug,
        )
        .await?;
    }
    Ok(())
}

async fn run_discord_menu(config: &Config) -> Result<()> {
    loop {
        clear_screen();
        let settings = config.get_discord_settings();
        let enabled_status = match &settings {
            Some(s) if s.enabled => "ON".to_string(),
            _ => "OFF".to_string(),
        };

        let rpc_status = match &settings {
            Some(s) if s.rpc_enabled => "ON".to_string(),
            None => "ON".to_string(),
            _ => "OFF".to_string(),
        };

        let client_id = match &settings {
            Some(s) => s.client_id.clone(),
            None => "1089228496459345970".to_string(),
        };

        let status_str = if enabled_status == "ON" {
            theme::style_primary("ON").to_string()
        } else {
            theme::style_dim("OFF").to_string()
        };

        let rpc_status_str = if rpc_status == "ON" {
            theme::style_primary("ON").to_string()
        } else {
            theme::style_dim("OFF").to_string()
        };

        println!("\n  ── 🤖 Discord Selfbot Mode (Jockie Music) ──");
        println!("  Status: {}", status_str);
        println!("  Discord Rich Presence (RPC): {}", rpc_status_str);
        println!("  RPC Client ID: {}\n", theme::style_primary(&client_id));

        let selections = vec![
            format!("Toggle Discord Mode (Currently: {})", enabled_status),
            "Configure Token & Channel ID".to_string(),
            format!("Toggle Rich Presence (Currently: {})", rpc_status),
            "Configure RPC Client ID".to_string(),
            "🔙 Go back".to_string(),
        ];

        let selection = dialoguer::Select::with_theme(&theme::get_dialoguer_theme())
            .with_prompt("Discord Bot Settings")
            .default(0)
            .items(&selections)
            .interact()?;

        match selection {
            0 => {
                let mut s = settings.clone().unwrap_or(crate::config::DiscordSettings {
                    enabled: false,
                    token: String::new(),
                    channel_id: String::new(),
                    rpc_enabled: true,
                    client_id: "1089228496459345970".to_string(),
                });

                if !s.enabled && (s.token.is_empty() || s.channel_id.is_empty()) {
                    println!("\n  ⚠️  Discord Selfbot credentials are required before enabling.");

                    let token: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                        .with_prompt("Enter Discord User Token (Selfbot)")
                        .interact_text()?;

                    let channel_id: String =
                        dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                            .with_prompt("Enter Target Discord Text Channel ID")
                            .interact_text()?;

                    let token = token.trim();
                    let channel_id = channel_id.trim();

                    if token.is_empty() || channel_id.is_empty() {
                        println!(
                            "  ❌ {}",
                            theme::style_error("Token and Channel ID cannot be empty.")
                        );
                        press_enter_to_continue();
                        continue;
                    }

                    s.token = token.to_string();
                    s.channel_id = channel_id.to_string();
                }

                s.enabled = !s.enabled;
                config.save_discord_settings(&s)?;

                if s.enabled {
                    println!(
                        "\n  ✅ Discord Selfbot Mode enabled! (Warning: Selfbots violate Discord TOS. Use at your own risk.)"
                    );
                } else {
                    println!("\n  ❌ Discord Selfbot Mode disabled.");
                }
                press_enter_to_continue();
            }
            1 => {
                let mut s = settings.unwrap_or(crate::config::DiscordSettings {
                    enabled: false,
                    token: String::new(),
                    channel_id: String::new(),
                    rpc_enabled: true,
                    client_id: "1089228496459345970".to_string(),
                });

                let token: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt("Enter Discord User Token")
                    .default(s.token)
                    .interact_text()?;

                let channel_id: String =
                    dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                        .with_prompt("Enter Discord Text Channel ID")
                        .default(s.channel_id)
                        .interact_text()?;

                let token = token.trim();
                let channel_id = channel_id.trim();

                if token.is_empty() || channel_id.is_empty() {
                    println!(
                        "  ❌ {}",
                        theme::style_error("Token and Channel ID cannot be empty.")
                    );
                    press_enter_to_continue();
                    continue;
                }

                s.token = token.to_string();
                s.channel_id = channel_id.to_string();
                config.save_discord_settings(&s)?;

                println!("\n  ✨ Credentials updated successfully.");
                press_enter_to_continue();
            }
            2 => {
                let mut s = settings.clone().unwrap_or(crate::config::DiscordSettings {
                    enabled: false,
                    token: String::new(),
                    channel_id: String::new(),
                    rpc_enabled: true,
                    client_id: "1089228496459345970".to_string(),
                });
                s.rpc_enabled = !s.rpc_enabled;
                config.save_discord_settings(&s)?;
                if s.rpc_enabled {
                    println!("\n  ✅ Discord Rich Presence enabled!");
                } else {
                    println!("\n  ❌ Discord Rich Presence disabled.");
                }
                press_enter_to_continue();
            }
            3 => {
                let mut s = settings.clone().unwrap_or(crate::config::DiscordSettings {
                    enabled: false,
                    token: String::new(),
                    channel_id: String::new(),
                    rpc_enabled: true,
                    client_id: "1089228496459345970".to_string(),
                });

                let client_id: String = dialoguer::Input::with_theme(&theme::get_dialoguer_theme())
                    .with_prompt("Enter Discord RPC Client ID")
                    .default(s.client_id)
                    .interact_text()?;

                let client_id = client_id.trim();
                if client_id.is_empty() {
                    println!("  ❌ {}", theme::style_error("Client ID cannot be empty."));
                    press_enter_to_continue();
                    continue;
                }

                s.client_id = client_id.to_string();
                config.save_discord_settings(&s)?;

                println!("\n  ✨ Client ID updated successfully.");
                press_enter_to_continue();
            }
            _ => break,
        }
    }
    Ok(())
}

async fn run_interactive_menu(
    config: &Config,
    db: &Db,
    client: &NetworkClient,
    current_volume: &mut f32,
    debug: bool,
) -> Result<()> {
    loop {
        clear_screen();
        theme::print_banner();
        let logged_in = config.is_logged_in();

        let lyrics_enabled = db
            .get_setting("lyrics_enabled")
            .ok()
            .flatten()
            .unwrap_or_else(|| "true".to_string())
            == "true";
        let lyrics_status = if lyrics_enabled {
            theme::style_primary("ON").to_string()
        } else {
            theme::style_dim("OFF").to_string()
        };

        let discord_settings = config.get_discord_settings();
        let discord_enabled = matches!(&discord_settings, Some(s) if s.enabled);
        let discord_status = if discord_enabled {
            theme::style_primary("ON").to_string()
        } else {
            theme::style_dim("OFF").to_string()
        };

        let mut selections = vec![
            "🔍 Search and Play (Tracks)".to_string(),
            "💿 Search Albums".to_string(),
            "📜 Playback History".to_string(),
            "💾 Cache Management".to_string(),
            format!("🎤 Lyrics Engine (Currently: {})", lyrics_status),
            format!("🤖 Discord Selfbot Mode (Currently: {})", discord_status),
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
            1 => run_search_albums_and_play(config, db, client, current_volume, debug).await?,
            2 => run_history(config, db, client, current_volume, debug).await?,
            3 => run_cache_menu(config, db).await?,
            4 => {
                let new_val = if lyrics_enabled { "false" } else { "true" };
                db.set_setting("lyrics_enabled", new_val)?;
                if new_val == "true" {
                    println!("  ✨ Lyrics engine activated.");
                } else {
                    println!("  ✨ Lyrics engine deactivated.");
                }
                press_enter_to_continue();
            }
            5 => run_discord_menu(config).await?,
            6 => {
                if logged_in {
                    run_library_playlists(config, db, client, current_volume, debug).await?;
                } else {
                    run_login_flow(config).await?;
                }
            }
            7 => {
                if logged_in {
                    config.logout()?;
                    println!("  ✨ Logged out successfully.");
                    press_enter_to_continue();
                } else {
                    println!("  Goodbye! 👋");
                    break;
                }
            }
            8 => {
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
        )
        .ok();
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
            let mut queue = vec![selected_track.clone()];
            let mut autoplay_ctoken = None;

            let discord_active = config
                .get_discord_settings()
                .map(|d| d.enabled)
                .unwrap_or(false);
            if !discord_active {
                println!("  📻 Fetching autoplay recommendations... ");
                match client.fetch_autoplay_queue(&selected_track.id).await {
                    Ok((mut autoplay_tracks, ctoken)) => {
                        queue.append(&mut autoplay_tracks);
                        autoplay_ctoken = ctoken;
                    }
                    Err(e) => {
                        log::warn!("Failed to fetch autoplay tracks: {}", e);
                    }
                }
            }
            play_queue(
                queue,
                0,
                &config,
                &db,
                &client,
                autoplay_ctoken,
                &mut current_volume,
                debug,
            )
            .await?;
        }
        Some(Commands::Album { query }) => {
            println!("Searching for album '{}'...", query);
            let albums = client.search_albums(&query).await?;
            if albums.is_empty() {
                println!("No matching album found.");
                return Ok(());
            }
            let selected_album = &albums[0];
            println!("Loading tracks for album '{}'...", selected_album.title);
            let album_url = format!(
                "https://music.youtube.com/playlist?list={}",
                selected_album.id
            );
            let tracks = fetch_playlist_with_retry(&config, &client, &album_url).await?;
            if tracks.is_empty() {
                println!("This album has no tracks.");
                return Ok(());
            }
            play_queue(
                tracks,
                0,
                &config,
                &db,
                &client,
                None,
                &mut current_volume,
                debug,
            )
            .await?;
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
        Some(Commands::SearchAlbum { query }) => {
            println!("Searching for albums matching '{}'...", query);
            let albums = client.search_albums(&query).await?;
            if albums.is_empty() {
                println!("No albums found.");
                return Ok(());
            }
            print_album_table(&albums);
        }
        Some(Commands::Cache { action }) => match action {
            CacheCommands::List => {
                let tracks = db.list_cached_tracks()?;
                if tracks.is_empty() {
                    println!("No cached tracks found.");
                    return Ok(());
                }
                println!(
                    "{:<12} | {:<30} | {:<20} | {:<8} | {}",
                    "ID", "Title", "Artist", "Duration", "Cached At"
                );
                println!("{}", "-".repeat(90));
                for track in tracks {
                    let dur_str = format_duration(Duration::from_secs(track.duration_secs as u64));
                    println!(
                        "{:<12} | {:<30} | {:<20} | {:<8} | {}",
                        track.video_id,
                        truncate(&track.title, 30),
                        truncate(&track.artist, 20),
                        dur_str,
                        track.cached_at
                    );
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
        },
        Some(Commands::History { limit }) => {
            let history = db.get_history(limit)?;
            if history.is_empty() {
                println!("No playback history found.");
                return Ok(());
            }
            println!(
                "{:<25} | {:<30} | {:<20} | {}",
                "Played At", "Title", "Artist", "Video ID"
            );
            println!("{}", "-".repeat(90));
            for entry in history {
                println!(
                    "{:<25} | {:<30} | {:<20} | {}",
                    entry.played_at,
                    truncate(&entry.title, 30),
                    truncate(&entry.artist, 20),
                    entry.video_id
                );
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
            println!("Fetching playlist details...");
            let mut tracks = fetch_playlist_with_retry(&config, &client, &url).await?;
            if tracks.is_empty() {
                println!("No tracks found in playlist.");
                return Ok(());
            }
            if shuffle {
                shuffle_tracks(&mut tracks);
            }
            play_queue(
                tracks,
                0,
                &config,
                &db,
                &client,
                None,
                &mut current_volume,
                debug,
            )
            .await?;
        }
    }

    Ok(())
}
