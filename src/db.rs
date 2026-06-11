use std::path::Path;
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, Result};

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
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                video_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                played_at TEXT NOT NULL
            );",
            [],
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn get_cached_track(&self, video_id: &str) -> Result<Option<CachedTrack>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT video_id, title, artist, duration_secs, file_path, cached_at 
             FROM cached_tracks WHERE video_id = ?1"
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
             FROM cached_tracks ORDER BY cached_at DESC"
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
             FROM history ORDER BY played_at DESC LIMIT ?1"
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
}
