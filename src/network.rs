use rustypipe::client::RustyPipe;
use rustypipe::model::paginator::ContinuationEndpoint;
use serde::Deserialize;

pub struct NetworkClient {
    client: RustyPipe,
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
struct YtDlpPlaylistEntry {
    id: Option<String>,
    title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    duration: Option<f64>,
}

#[derive(Deserialize)]
struct YtDlpPlaylistDump {
    entries: Option<Vec<YtDlpPlaylistEntry>>,
}

#[derive(Deserialize)]
struct YtDlpLibraryPlaylistEntry {
    id: Option<String>,
    title: Option<String>,
    playlist_count: Option<usize>,
}

#[derive(Deserialize)]
struct YtDlpLibraryDump {
    entries: Option<Vec<YtDlpLibraryPlaylistEntry>>,
}

impl NetworkClient {
    pub fn new() -> Self {
        Self {
            client: RustyPipe::new(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<TrackInfo>, anyhow::Error> {
        let search_result = self.client.query()
            .music_search_tracks(query)
            .await
            .map_err(|e| anyhow::anyhow!("Search error: {:?}", e))?;
        
        let mut tracks = Vec::new();
        for track in search_result.items.items {
            let artist_name = track.artists
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
    ) -> Result<String, anyhow::Error> {
        let output = tokio::process::Command::new(yt_dlp_path)
            .args(&[
                "--no-warnings",
                "--js-runtimes", js_runtime,
                "--remote-components", "ejs:github",
                "-g",
                "-f",
                "ba[ext=m4a]/ba",
                &format!("https://www.youtube.com/watch?v={}", video_id),
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("yt-dlp failed to extract stream URL: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let url = stdout.trim().to_string();
        if url.is_empty() {
            return Err(anyhow::anyhow!("yt-dlp returned empty stream URL"));
        }

        Ok(url)
    }

    pub async fn fetch_playlist(
        &self,
        yt_dlp_path: &std::path::Path,
        browser: Option<&str>,
        cookies_path: &std::path::Path,
        js_runtime: &str,
        playlist_url: &str,
    ) -> Result<Vec<TrackInfo>, anyhow::Error> {
        let mut cmd = tokio::process::Command::new(yt_dlp_path);
        cmd.args(&[
            "--no-warnings",
            "--js-runtimes", js_runtime,
            "--remote-components", "ejs:github",
            "--flat-playlist",
            "-J",
        ]);
        
        if let Some(b) = browser {
            cmd.arg("--cookies-from-browser").arg(b);
        } else if cookies_path.exists() && std::fs::metadata(cookies_path).map(|m| m.len() > 0).unwrap_or(false) {
            cmd.arg("--cookies").arg(cookies_path);
        }
        
        cmd.arg(playlist_url);
        
        // Ensure child process is killed if we abort the task
        cmd.kill_on_drop(true);
        
        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to fetch playlist details: {}", stderr));
        }

        let dump: YtDlpPlaylistDump = serde_json::from_slice(&output.stdout)?;
        let mut tracks = Vec::new();

        if let Some(entries) = dump.entries {
            for entry in entries {
                let id = match entry.id {
                    Some(id) if !id.is_empty() => id,
                    _ => continue,
                };

                let title = entry.title.unwrap_or_else(|| "Unknown Title".to_string());
                let artist = entry.uploader
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

        Ok(tracks)
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
            "--js-runtimes", js_runtime,
            "--remote-components", "ejs:github",
            "--flat-playlist",
            "-J",
        ]);
        
        if let Some(b) = browser {
            cmd.arg("--cookies-from-browser").arg(b);
        } else if cookies_path.exists() && std::fs::metadata(cookies_path).map(|m| m.len() > 0).unwrap_or(false) {
            cmd.arg("--cookies").arg(cookies_path);
        } else {
            return Err(anyhow::anyhow!("Not logged in (browser.txt or cookies.txt not found). Use the login feature first."));
        }
        
        cmd.arg("https://www.youtube.com/feed/playlists");
        
        // Ensure child process is killed if we abort the task
        cmd.kill_on_drop(true);
        
        let output = cmd.output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to fetch library playlists: {}", stderr));
        }

        let dump: YtDlpLibraryDump = serde_json::from_slice(&output.stdout)?;
        let mut playlists = Vec::new();

        if let Some(entries) = dump.entries {
            for entry in entries {
                let id = match entry.id {
                    Some(id) if !id.is_empty() => id,
                    _ => continue,
                };

                let title = entry.title.unwrap_or_else(|| "Untitled Playlist".to_string());
                let track_count = entry.playlist_count;

                playlists.push(PlaylistInfo {
                    id,
                    title,
                    track_count,
                });
            }
        }

        // Add Liked Music at the top
        playlists.insert(0, PlaylistInfo {
            id: "LM".to_string(),
            title: "Liked Music".to_string(),
            track_count: None,
        });

        Ok(playlists)
    }

    pub async fn fetch_autoplay_queue(
        &self,
        video_id: &str,
    ) -> Result<(Vec<TrackInfo>, Option<(String, ContinuationEndpoint)>), anyhow::Error> {
        let radio = self.client.query()
            .music_radio_track(video_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch autoplay tracks: {:?}", e))?;
        
        let mut tracks = Vec::new();
        for item in radio.items {
            let artist_name = item.artists
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
        let paginator = query.continuation::<rustypipe::model::TrackItem, _>(ctoken, endpoint, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch continuation tracks: {:?}", e))?;
        
        let mut tracks = Vec::new();
        for item in paginator.items {
            let artist_name = item.artists
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
        let search_result = self.client.query()
            .music_search_albums(query)
            .await
            .map_err(|e| anyhow::anyhow!("Search error: {:?}", e))?;
        
        let mut albums = Vec::new();
        for album in search_result.items.items {
            let artist_name = album.artists
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
