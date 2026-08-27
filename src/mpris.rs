use mpris_server::zbus::{self, fdo};
use mpris_server::{
    Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Time, TrackId,
};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Clone)]
pub enum MprisCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    Seek(i64),
    SetPosition(i64),
    SetVolume(f64),
}

#[derive(Clone)]
pub struct MprisHandler {
    cmd_tx: mpsc::UnboundedSender<MprisCommand>,
}

impl MprisHandler {
    pub fn new(cmd_tx: mpsc::UnboundedSender<MprisCommand>) -> Self {
        Self { cmd_tx }
    }
}

impl RootInterface for MprisHandler {
    async fn identity(&self) -> fdo::Result<String> {
        Ok("YTM-CLI".to_string())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("ytm-cli".to_string())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["http".to_string(), "https".to_string()])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![
            "audio/mp4".to_string(),
            "audio/flac".to_string(),
            "audio/webm".to_string(),
        ])
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn quit(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Stop);
        Ok(())
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
}

impl PlayerInterface for MprisHandler {
    async fn next(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::PlayPause);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Play);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Seek(offset.as_micros()));
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, position: Time) -> fdo::Result<()> {
        let _ = self
            .cmd_tx
            .send(MprisCommand::SetPosition(position.as_micros()));
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(PlaybackStatus::Playing)
    }

    async fn loop_status(&self) -> fdo::Result<mpris_server::LoopStatus> {
        Ok(mpris_server::LoopStatus::None)
    }

    async fn set_loop_status(&self, _loop_status: mpris_server::LoopStatus) -> zbus::Result<()> {
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: f64) -> zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(Metadata::new())
    }

    async fn volume(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn set_volume(&self, volume: f64) -> zbus::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::SetVolume(volume));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros(0))
    }

    async fn minimum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

pub struct MprisManager {
    server: Option<Arc<Mutex<Server<MprisHandler>>>>,
}

impl MprisManager {
    pub async fn start(cmd_tx: mpsc::UnboundedSender<MprisCommand>) -> Self {
        let handler = MprisHandler::new(cmd_tx);
        let pid = std::process::id();
        let name = format!("ytm_cli_instance_{}", pid);
        match Server::new(&name, handler).await {
            Ok(server) => Self {
                server: Some(Arc::new(Mutex::new(server))),
            },
            Err(e) => {
                log::debug!("Failed to start MPRIS server: {}", e);
                Self { server: None }
            }
        }
    }

    pub async fn update_metadata(
        &self,
        title: &str,
        artist: &str,
        video_id: &str,
        duration_secs: Option<u32>,
    ) {
        if let Some(ref s) = self.server {
            let mut metadata = Metadata::new();
            metadata.set_title(Some(title));
            metadata.set_artist(Some([artist]));
            metadata.set_url(Some(format!(
                "https://www.youtube.com/watch?v={}",
                video_id
            )));
            if let Some(dur) = duration_secs {
                metadata.set_length(Some(Time::from_micros(dur as i64 * 1_000_000)));
            }
            let server = s.lock().await;
            let _ = server
                .properties_changed([Property::Metadata(metadata)])
                .await;
        }
    }

    pub async fn set_playback_status(&self, is_paused: bool) {
        if let Some(ref s) = self.server {
            let status = if is_paused {
                PlaybackStatus::Paused
            } else {
                PlaybackStatus::Playing
            };
            let server = s.lock().await;
            let _ = server
                .properties_changed([Property::PlaybackStatus(status)])
                .await;
        }
    }

    pub async fn set_playback_stopped(&self) {
        if let Some(ref s) = self.server {
            let server = s.lock().await;
            let _ = server
                .properties_changed([Property::PlaybackStatus(PlaybackStatus::Stopped)])
                .await;
        }
    }
}
