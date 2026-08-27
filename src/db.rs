use rusqlite::{Connection, Result, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct CachedTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: u32,
    pub file_path: String,
    pub cached_at: String,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i32,
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub played_at: String,
}

#[derive(Debug, Clone)]
pub struct FavoriteTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: u32,
    pub added_at: String,
}

#[derive(Debug, Clone)]
pub struct LocalPlaylist {
    pub id: i64,
    pub name: String,
    pub created_at: String,
    pub track_count: usize,
}

#[derive(Debug, Clone)]
pub struct LocalPlaylistTrack {
    pub id: i64,
    pub playlist_id: i64,
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: u32,
    pub position: i32,
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Initialize tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cached_tracks (
                video_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                cached_at TEXT NOT NULL
            );",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                played_at TEXT NOT NULL
            );",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS favorites (
                video_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                added_at TEXT NOT NULL
            );",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS local_playlists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            );",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS local_playlist_tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                playlist_id INTEGER NOT NULL,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                position INTEGER NOT NULL,
                FOREIGN KEY(playlist_id) REFERENCES local_playlists(id) ON DELETE CASCADE
            );",
            [],
        )?;

        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn get_cached_track(&self, video_id: &str) -> Result<Option<CachedTrack>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, file_path, cached_at 
             FROM cached_tracks WHERE video_id = ?1",
        )?;

        let mut rows = stmt.query(params![video_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(CachedTrack {
                video_id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                duration_secs: row.get(3)?,
                file_path: row.get(4)?,
                cached_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_cached_track(
        &self,
        video_id: &str,
        title: &str,
        artist: &str,
        duration_secs: u32,
        file_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO cached_tracks (video_id, title, artist, duration_secs, file_path, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![video_id, title, artist, duration_secs, file_path, now],
        )?;
        Ok(())
    }

    pub fn list_cached_tracks(&self) -> Result<Vec<CachedTrack>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, file_path, cached_at 
             FROM cached_tracks ORDER BY cached_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(CachedTrack {
                video_id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                duration_secs: row.get(3)?,
                file_path: row.get(4)?,
                cached_at: row.get(5)?,
            })
        })?;

        let mut tracks = Vec::new();
        for track in rows {
            tracks.push(track?);
        }
        Ok(tracks)
    }

    pub fn delete_cached_track(&self, video_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM cached_tracks WHERE video_id = ?1",
            params![video_id],
        )?;
        Ok(())
    }

    pub fn clear_cache(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM cached_tracks", [])?;
        Ok(())
    }

    pub fn add_history(&self, video_id: &str, title: &str, artist: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO history (video_id, title, artist, played_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![video_id, title, artist, now],
        )?;
        Ok(())
    }

    pub fn get_history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, video_id, title, artist, played_at 
             FROM history ORDER BY played_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                video_id: row.get(1)?,
                title: row.get(2)?,
                artist: row.get(3)?,
                played_at: row.get(4)?,
            })
        })?;

        let mut history = Vec::new();
        for entry in rows {
            history.push(entry?);
        }
        Ok(history)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_max_cache_size_bytes(&self) -> u64 {
        let mb = self
            .get_setting("max_cache_size_mb")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(50); // Default to 50mb
        mb * 1024 * 1024
    }

    pub fn update_cached_track_accessed(&self, video_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE cached_tracks SET cached_at = ?1 WHERE video_id = ?2",
            params![now, video_id],
        )?;
        Ok(())
    }

    pub fn enforce_cache_limit(&self, _cache_dir: &Path, max_bytes: u64) -> Result<()> {
        // Retrieve all cached tracks ordered by cached_at ASC (oldest first)
        let tracks = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT video_id, file_path FROM cached_tracks ORDER BY cached_at ASC")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;

            let mut tracks = Vec::new();
            for r in rows {
                tracks.push(r?);
            }
            tracks
        };

        // Calculate total size of all cached files
        let mut total_size: u64 = 0;
        let mut file_sizes = Vec::new();
        for (video_id, file_path) in &tracks {
            let path = Path::new(file_path);
            let size = if path.exists() {
                path.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            total_size += size;
            file_sizes.push((video_id.clone(), file_path.clone(), size));
        }

        // Evict files starting from the oldest until total_size <= max_bytes
        for (video_id, file_path, size) in file_sizes {
            if total_size <= max_bytes {
                break;
            }
            // Evict this track
            let path = Path::new(&file_path);
            if path.exists() {
                std::fs::remove_file(path).ok();
            }
            self.delete_cached_track(&video_id)?;
            total_size = total_size.saturating_sub(size);
        }

        Ok(())
    }

    // --- Favorites ---
    pub fn add_favorite(
        &self,
        video_id: &str,
        title: &str,
        artist: &str,
        duration_secs: u32,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO favorites (video_id, title, artist, duration_secs, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![video_id, title, artist, duration_secs, now],
        )?;
        Ok(())
    }

    pub fn remove_favorite(&self, video_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM favorites WHERE video_id = ?1",
            params![video_id],
        )?;
        Ok(())
    }

    pub fn is_favorite(&self, video_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT 1 FROM favorites WHERE video_id = ?1")?;
        let exists = stmt.exists(params![video_id])?;
        Ok(exists)
    }

    pub fn list_favorites(&self) -> Result<Vec<FavoriteTrack>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, added_at
             FROM favorites ORDER BY added_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FavoriteTrack {
                video_id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                duration_secs: row.get(3)?,
                added_at: row.get(4)?,
            })
        })?;

        let mut favs = Vec::new();
        for f in rows {
            favs.push(f?);
        }
        Ok(favs)
    }

    // --- Local Playlists ---
    pub fn create_local_playlist(&self, name: &str) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO local_playlists (name, created_at) VALUES (?1, ?2)",
            params![name.trim(), now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_local_playlist(&self, playlist_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM local_playlists WHERE id = ?1",
            params![playlist_id],
        )?;
        Ok(())
    }

    pub fn list_local_playlists(&self) -> Result<Vec<LocalPlaylist>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.name, p.created_at, COUNT(t.id) as track_count
             FROM local_playlists p
             LEFT JOIN local_playlist_tracks t ON p.id = t.playlist_id
             GROUP BY p.id, p.name, p.created_at
             ORDER BY p.id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(LocalPlaylist {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                track_count: row.get::<_, i64>(3)? as usize,
            })
        })?;

        let mut playlists = Vec::new();
        for p in rows {
            playlists.push(p?);
        }
        Ok(playlists)
    }

    pub fn add_track_to_local_playlist(
        &self,
        playlist_id: i64,
        video_id: &str,
        title: &str,
        artist: &str,
        duration_secs: u32,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let max_pos: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), 0) FROM local_playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |r| r.get(0),
            )
            .unwrap_or(0);

        conn.execute(
            "INSERT INTO local_playlist_tracks (playlist_id, video_id, title, artist, duration_secs, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![playlist_id, video_id, title, artist, duration_secs, max_pos + 1],
        )?;
        Ok(())
    }

    pub fn remove_track_from_local_playlist(&self, track_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM local_playlist_tracks WHERE id = ?1",
            params![track_id],
        )?;
        Ok(())
    }

    pub fn get_local_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<LocalPlaylistTrack>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, playlist_id, video_id, title, artist, duration_secs, position
             FROM local_playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![playlist_id], |row| {
            Ok(LocalPlaylistTrack {
                id: row.get(0)?,
                playlist_id: row.get(1)?,
                video_id: row.get(2)?,
                title: row.get(3)?,
                artist: row.get(4)?,
                duration_secs: row.get(5)?,
                position: row.get(6)?,
            })
        })?;

        let mut tracks = Vec::new();
        for t in rows {
            tracks.push(t?);
        }
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cached_tracks (
                video_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                cached_at TEXT NOT NULL
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                played_at TEXT NOT NULL
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS favorites (
                video_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                added_at TEXT NOT NULL
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS local_playlists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS local_playlist_tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                playlist_id INTEGER NOT NULL,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_secs INTEGER NOT NULL,
                position INTEGER NOT NULL,
                FOREIGN KEY(playlist_id) REFERENCES local_playlists(id) ON DELETE CASCADE
            );",
            [],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        Db {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    #[test]
    fn test_favorites_crud() {
        let db = create_test_db();
        assert!(!db.is_favorite("song1").unwrap());

        db.add_favorite("song1", "Song Title 1", "Artist 1", 180)
            .unwrap();
        assert!(db.is_favorite("song1").unwrap());

        let favs = db.list_favorites().unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].video_id, "song1");
        assert_eq!(favs[0].title, "Song Title 1");

        db.remove_favorite("song1").unwrap();
        assert!(!db.is_favorite("song1").unwrap());
        assert_eq!(db.list_favorites().unwrap().len(), 0);
    }

    #[test]
    fn test_local_playlists_crud() {
        let db = create_test_db();

        let pl_id = db.create_local_playlist("My Gym Beats").unwrap();
        assert!(pl_id > 0);

        let playlists = db.list_local_playlists().unwrap();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].name, "My Gym Beats");
        assert_eq!(playlists[0].track_count, 0);

        db.add_track_to_local_playlist(pl_id, "gym_song_1", "Workout Anthem", "Pump", 200)
            .unwrap();
        db.add_track_to_local_playlist(pl_id, "gym_song_2", "Beast Mode", "Gainz", 195)
            .unwrap();

        let playlists_updated = db.list_local_playlists().unwrap();
        assert_eq!(playlists_updated[0].track_count, 2);

        let tracks = db.get_local_playlist_tracks(pl_id).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "Workout Anthem");
        assert_eq!(tracks[1].title, "Beast Mode");

        // Remove track
        db.remove_track_from_local_playlist(tracks[0].id).unwrap();
        let tracks_after_removal = db.get_local_playlist_tracks(pl_id).unwrap();
        assert_eq!(tracks_after_removal.len(), 1);
        assert_eq!(tracks_after_removal[0].title, "Beast Mode");

        // Cascade delete playlist
        db.delete_local_playlist(pl_id).unwrap();
        assert_eq!(db.list_local_playlists().unwrap().len(), 0);
        assert_eq!(db.get_local_playlist_tracks(pl_id).unwrap().len(), 0);
    }
}
