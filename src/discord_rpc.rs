use crate::config::Config;
use crate::network::TrackInfo;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CLIENT_ID: &str = "1089228496459345970"; // Public YouTube Music RPC Client ID

pub struct DiscordRpc {
    client: Option<DiscordIpcClient>,
    enabled: bool,
}

impl DiscordRpc {
    pub fn new(config: &Config) -> Self {
        let enabled = config
            .get_discord_settings()
            .map(|s| s.rpc_enabled)
            .unwrap_or(true);

        Self {
            client: None,
            enabled,
        }
    }

    pub fn connect(&mut self) {
        if !self.enabled {
            return;
        }

        let mut client = match DiscordIpcClient::new(CLIENT_ID) {
            Ok(c) => c,
            Err(_) => return,
        };

        if client.connect().is_ok() {
            self.client = Some(client);
        }
    }

    pub fn update(&mut self, track: &TrackInfo, elapsed: Duration, is_paused: bool) {
        let client = match &mut self.client {
            Some(c) => c,
            None => return,
        };

        let mut act = activity::Activity::new().details(&track.title).assets(
            activity::Assets::new()
                .large_image("ytm")
                .large_text("YTM-CLI"),
        );

        let state_str;
        if is_paused {
            state_str = format!("⏸️ {}", track.artist);
            act = act.state(&state_str);
        } else {
            act = act.state(&track.artist);

            // Set timestamps
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let start_secs = now_secs - elapsed.as_secs() as i64;

            let mut ts = activity::Timestamps::new().start(start_secs);
            if let Some(duration_secs) = track.duration_secs {
                let end_secs = start_secs + duration_secs as i64;
                ts = ts.end(end_secs);
            }
            act = act.timestamps(ts);
        }

        let _ = client.set_activity(act);
    }

    pub fn clear(&mut self) {
        if let Some(ref mut client) = self.client {
            let _ = client.clear_activity();
        }
    }
}
