use rustypipe::client::RustyPipe;
use rustypipe::model::paginator::ContinuationEndpoint;
use serde::Deserialize;

pub struct NetworkClient {
    client: RustyPipe,
    cookies_loaded: std::sync::atomic::AtomicBool,
    cookies_expired: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct AlbumInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub track_count: Option<usize>,
}

#[derive(Deserialize)]
struct YtDlpLibraryDump {
    entries: Option<Vec<YtDlpLibraryPlaylistEntry>>,
}

#[derive(Deserialize)]
struct YtDlpLibraryPlaylistEntry {
    id: Option<String>,
    title: Option<String>,
    playlist_count: Option<usize>,
}

impl NetworkClient {
    pub fn new() -> Self {
        Self {
            client: RustyPipe::new(),
            cookies_loaded: std::sync::atomic::AtomicBool::new(false),
            cookies_expired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<TrackInfo>, anyhow::Error> {
        let search_result = self
            .client
            .query()
            .music_search_tracks(query)
            .await
            .map_err(|e| anyhow::anyhow!("Search error: {:?}", e))?;

        let mut tracks = Vec::new();
        for track in search_result.items.items {
            let artist_name = track
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            tracks.push(TrackInfo {
                id: track.id,
                title: track.name,
                artist: artist_name,
                duration_secs: track.duration,
            });
        }
        Ok(tracks)
    }

    pub async fn get_stream_url(
        &self,
        video_id: &str,
        yt_dlp_path: &std::path::Path,
        js_runtime: &str,
        cookies_path: Option<&std::path::Path>,
        browser: Option<&str>,
    ) -> Result<String, anyhow::Error> {
        // 1. Try extracting WITHOUT cookies first
        let mut cmd = tokio::process::Command::new(yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes",
            js_runtime,
            "--remote-components",
            "ejs:github",
            "-g",
            "-f",
            "ba[ext=m4a]/ba",
            &format!("https://www.youtube.com/watch?v={}", video_id),
        ]);

        let output = cmd.output().await?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let url = stdout.trim().to_string();
            if !url.is_empty() {
                return Ok(url);
            }
        }

        // 2. If it fails, retry WITH cookies
        let mut cmd = tokio::process::Command::new(yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes",
            js_runtime,
            "--remote-components",
            "ejs:github",
            "-g",
            "-f",
            "ba[ext=m4a]/ba",
            &format!("https://www.youtube.com/watch?v={}", video_id),
        ]);
        if let Some(b) = browser {
            cmd.arg("--cookies-from-browser").arg(b);
        } else if let Some(cp) = cookies_path {
            if cp.exists() && std::fs::metadata(cp).map(|m| m.len() > 0).unwrap_or(false) {
                cmd.arg("--cookies").arg(cp);
            }
        }

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "yt-dlp failed to extract stream URL: {}",
                stderr
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let url = stdout.trim().to_string();
        if url.is_empty() {
            return Err(anyhow::anyhow!("yt-dlp returned empty stream URL"));
        }
        Ok(url)
    }

    pub async fn reset_cookies(&self) {
        self.cookies_loaded
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.cookies_expired
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = self.client.user_auth_remove_cookie().await;
    }

    pub async fn load_cookies(
        &self,
        yt_dlp_path: &std::path::Path,
        browser: Option<&str>,
        cookies_path: &std::path::Path,
    ) -> Result<(), anyhow::Error> {
        if self
            .cookies_loaded
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(());
        }

        let mut success = false;
        let mut has_cookies_configured = false;

        if let Some(b) = browser {
            has_cookies_configured = true;
            let temp_cookies = std::env::temp_dir().join("ytm_cli_cookies_temp.txt");
            let _ = tokio::process::Command::new(yt_dlp_path)
                .args(&[
                    "--cookies-from-browser",
                    b,
                    "--cookies",
                    temp_cookies.to_str().unwrap(),
                    "--skip-download",
                    "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                ])
                .output()
                .await;
            if temp_cookies.exists()
                && std::fs::metadata(&temp_cookies)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
            {
                if let Ok(cookie_content) = std::fs::read_to_string(&temp_cookies) {
                    if self
                        .client
                        .user_auth_set_cookie_txt(&cookie_content)
                        .await
                        .is_ok()
                    {
                        success = true;
                    }
                }
                let _ = std::fs::remove_file(temp_cookies);
            }
        } else if cookies_path.exists()
            && std::fs::metadata(cookies_path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            has_cookies_configured = true;
            if let Ok(cookie_content) = std::fs::read_to_string(cookies_path) {
                if self
                    .client
                    .user_auth_set_cookie_txt(&cookie_content)
                    .await
                    .is_ok()
                {
                    success = true;
                }
            }
        }

        if has_cookies_configured && !success {
            self.cookies_expired
                .store(true, std::sync::atomic::Ordering::SeqCst);
        } else {
            self.cookies_expired
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }

        self.cookies_loaded
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    pub async fn fetch_playlist_ytdlp(
        yt_dlp_path: &std::path::Path,
        browser: Option<&str>,
        cookies_path: &std::path::Path,
        js_runtime: &str,
        playlist_url: &str,
    ) -> Result<Vec<TrackInfo>, anyhow::Error> {
        let mut cmd = tokio::process::Command::new(yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes",
            js_runtime,
            "--remote-components",
            "ejs:github",
            "--flat-playlist",
            "-J",
        ]);

        if let Some(b) = browser {
            cmd.arg("--cookies-from-browser").arg(b);
        } else if cookies_path.exists()
            && std::fs::metadata(cookies_path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            cmd.arg("--cookies").arg(cookies_path);
        }

        let full_url = if playlist_url.starts_with("http") {
            playlist_url.to_string()
        } else {
            format!("https://www.youtube.com/playlist?list={}", playlist_url)
        };

        cmd.arg(&full_url);
        cmd.kill_on_drop(true);

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("login") || stderr.contains("cookie") || stderr.contains("private") {
                return Err(anyhow::anyhow!("SESSION_EXPIRED"));
            }
            return Err(anyhow::anyhow!("yt-dlp playlist fetch failed: {}", stderr));
        }

        #[derive(Deserialize)]
        struct YtDlpPlaylistDump {
            entries: Option<Vec<YtDlpPlaylistItem>>,
        }

        #[derive(Deserialize)]
        struct YtDlpPlaylistItem {
            id: Option<String>,
            title: Option<String>,
            uploader: Option<String>,
            channel: Option<String>,
            duration: Option<f64>,
        }

        let dump: YtDlpPlaylistDump = serde_json::from_slice(&output.stdout)?;
        let mut tracks = Vec::new();

        if let Some(entries) = dump.entries {
            for entry in entries {
                let id = match entry.id {
                    Some(id) if !id.is_empty() => id,
                    _ => continue,
                };
                let title = entry.title.unwrap_or_else(|| "Untitled Track".to_string());
                let artist = entry
                    .uploader
                    .or(entry.channel)
                    .unwrap_or_else(|| "Unknown Artist".to_string());
                let duration_secs = entry.duration.map(|d| d.round() as u32);

                tracks.push(TrackInfo {
                    id,
                    title,
                    artist,
                    duration_secs,
                });
            }
        }

        if tracks.is_empty() {
            return Err(anyhow::anyhow!("No tracks found in playlist."));
        }

        Ok(tracks)
    }

    pub async fn fetch_playlist(
        &self,
        yt_dlp_path: &std::path::Path,
        browser: Option<&str>,
        cookies_path: &std::path::Path,
        js_runtime: &str,
        playlist_url: &str,
    ) -> Result<Vec<TrackInfo>, anyhow::Error> {
        let _ = self.load_cookies(yt_dlp_path, browser, cookies_path).await;

        let playlist_id = if playlist_url.starts_with("http") {
            if let Some(pos) = playlist_url.find("list=") {
                let id = &playlist_url[pos + 5..];
                if let Some(end) = id.find('&') {
                    id[..end].to_string()
                } else {
                    id.to_string()
                }
            } else {
                playlist_url.to_string()
            }
        } else {
            playlist_url.to_string()
        };

        let res = if playlist_id.starts_with("MPREb_") {
            let album = self
                .client
                .query()
                .music_album(&playlist_id)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to fetch album details: {:?}", e))?;

            let artist_name = album
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown Artist".to_string());

            let tracks = album
                .tracks
                .into_iter()
                .map(|track| {
                    let track_artist = track
                        .artists
                        .first()
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| artist_name.clone());
                    TrackInfo {
                        id: track.id,
                        title: track.name,
                        artist: track_artist,
                        duration_secs: track.duration,
                    }
                })
                .collect();
            Ok(tracks)
        } else {
            match self.client.query().music_playlist(&playlist_id).await {
                Ok(mut playlist) => {
                    let _ = playlist.tracks.extend_all(self.client.query()).await;
                    let tracks = playlist
                        .tracks
                        .items
                        .into_iter()
                        .map(|track| {
                            let artist_name = track
                                .artists
                                .first()
                                .map(|a| a.name.clone())
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            TrackInfo {
                                id: track.id,
                                title: track.name,
                                artist: artist_name,
                                duration_secs: track.duration,
                            }
                        })
                        .collect();
                    Ok(tracks)
                }
                Err(e) => {
                    log::warn!(
                        "rustypipe failed to fetch playlist '{}': {:?}. Falling back to yt-dlp...",
                        playlist_id,
                        e
                    );
                    Self::fetch_playlist_ytdlp(
                        yt_dlp_path,
                        browser,
                        cookies_path,
                        js_runtime,
                        playlist_url,
                    )
                    .await
                }
            }
        };

        match res {
            Ok(tracks) => Ok(tracks),
            Err(e) => {
                if self
                    .cookies_expired
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    Err(anyhow::anyhow!("SESSION_EXPIRED"))
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn fetch_library_playlists(
        &self,
        yt_dlp_path: &std::path::Path,
        browser: Option<&str>,
        cookies_path: &std::path::Path,
        js_runtime: &str,
    ) -> Result<Vec<PlaylistInfo>, anyhow::Error> {
        let mut cmd = tokio::process::Command::new(yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes",
            js_runtime,
            "--remote-components",
            "ejs:github",
            "--flat-playlist",
            "-J",
        ]);

        if let Some(b) = browser {
            cmd.arg("--cookies-from-browser").arg(b);
        } else if cookies_path.exists()
            && std::fs::metadata(cookies_path)
                .map(|m| m.len() > 0)
                .unwrap_or(false)
        {
            cmd.arg("--cookies").arg(cookies_path);
        } else {
            return Err(anyhow::anyhow!(
                "Not logged in (browser.txt or cookies.txt not found). Use the login feature first."
            ));
        }

        cmd.arg("https://www.youtube.com/feed/playlists");

        // Ensure child process is killed if we abort the task
        cmd.kill_on_drop(true);

        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "Failed to fetch library playlists: {}",
                stderr
            ));
        }

        let dump: YtDlpLibraryDump = serde_json::from_slice(&output.stdout)?;
        let mut playlists = Vec::new();

        if let Some(entries) = dump.entries {
            for entry in entries {
                let id = match entry.id {
                    Some(id) if !id.is_empty() => id,
                    _ => continue,
                };

                let title = entry
                    .title
                    .unwrap_or_else(|| "Untitled Playlist".to_string());
                let track_count = entry.playlist_count;

                playlists.push(PlaylistInfo {
                    id,
                    title,
                    track_count,
                });
            }
        }

        // Add Liked Music at the top
        playlists.insert(
            0,
            PlaylistInfo {
                id: "LM".to_string(),
                title: "Liked Music".to_string(),
                track_count: None,
            },
        );

        Ok(playlists)
    }

    pub async fn fetch_autoplay_queue(
        &self,
        video_id: &str,
    ) -> Result<(Vec<TrackInfo>, Option<(String, ContinuationEndpoint)>), anyhow::Error> {
        let radio = self
            .client
            .query()
            .music_radio_track(video_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch autoplay tracks: {:?}", e))?;

        let mut tracks = Vec::new();
        for item in radio.items {
            let artist_name = item
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            tracks.push(TrackInfo {
                id: item.id,
                title: item.name,
                artist: artist_name,
                duration_secs: item.duration,
            });
        }

        let continuation = radio.ctoken.map(|tok| (tok, radio.endpoint));
        Ok((tracks, continuation))
    }

    pub async fn fetch_next_autoplay_page(
        &self,
        ctoken: &str,
        endpoint: ContinuationEndpoint,
    ) -> Result<(Vec<TrackInfo>, Option<String>), anyhow::Error> {
        let query = self.client.query();
        let paginator = query
            .continuation::<rustypipe::model::TrackItem, _>(ctoken, endpoint, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch continuation tracks: {:?}", e))?;

        let mut tracks = Vec::new();
        for item in paginator.items {
            let artist_name = item
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            tracks.push(TrackInfo {
                id: item.id,
                title: item.name,
                artist: artist_name,
                duration_secs: item.duration,
            });
        }

        Ok((tracks, paginator.ctoken))
    }

    pub async fn search_albums(&self, query: &str) -> Result<Vec<AlbumInfo>, anyhow::Error> {
        let search_result = self
            .client
            .query()
            .music_search_albums(query)
            .await
            .map_err(|e| anyhow::anyhow!("Search error: {:?}", e))?;

        let mut albums = Vec::new();
        for album in search_result.items.items {
            let artist_name = album
                .artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            albums.push(AlbumInfo {
                id: album.id,
                title: album.name,
                artist: artist_name,
                year: album.year,
            });
        }
        Ok(albums)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_utopia() {
        let client = NetworkClient::new();
        let album = client.client.query().music_album("MPREb_B7NkMWS9hMM").await;
        match &album {
            Ok(a) => {
                println!("--- UTOPIA Album Info ---");
                println!("Name: {}", a.name);
                println!("Tracks: {}", a.tracks.len());
                for track in &a.tracks {
                    println!("  - {} (id: {})", track.name, track.id);
                }
            }
            Err(e) => {
                println!("Failed to fetch UTOPIA: {:?}", e);
            }
        }
        assert!(album.is_ok());
    }

    #[tokio::test]
    async fn test_fetch_playlist_ytdlp() {
        let config = crate::config::Config::new();
        let yt_dlp = match config.ensure_yt_dlp().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let browser = config.get_cookie_browser_arg();
        let js_runtime = config.get_js_runtime_arg();

        let tracks = NetworkClient::fetch_playlist_ytdlp(
            &yt_dlp,
            browser.as_deref(),
            &config.cookies_path,
            &js_runtime,
            "https://www.youtube.com/playlist?list=PLPy6Ka57myt782w17YOhrAI1yXx79u6vC",
        )
        .await;

        if let Ok(ref t) = tracks {
            println!("--- Test Playlist Tracks: {} ---", t.len());
            for track in t.iter().take(3) {
                println!(
                    "  - {} by {} ({:?})",
                    track.title, track.artist, track.duration_secs
                );
            }
        }
        assert!(tracks.is_ok(), "Expected yt-dlp playlist fetch to succeed");
    }
}
