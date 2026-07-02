use crate::config::Config;
use crate::network::TrackInfo;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    pub fn connect(&mut self, client_id: &str) -> Result<(), anyhow::Error> {
        log::debug!(
            "DiscordRpc::connect called (enabled = {}, client_id = {})",
            self.enabled,
            client_id
        );
        if !self.enabled {
            return Err(anyhow::anyhow!("Discord RPC is disabled in configuration"));
        }

        let mut client = DiscordIpcClient::new(client_id)
            .map_err(|e| anyhow::anyhow!("Failed to create DiscordIpcClient: {}", e))?;

        client
            .connect()
            .map_err(|e| anyhow::anyhow!("Failed to connect to Discord RPC socket: {}", e))?;

        log::info!("Successfully connected to Discord RPC");
        self.client = Some(client);
        Ok(())
    }

    pub fn update(&mut self, track: &TrackInfo, elapsed: Duration, is_paused: bool) {
        let client = match &mut self.client {
            Some(c) => c,
            None => {
                log::debug!("DiscordRpc::update called, but client is not connected");
                return;
            }
        };

        log::debug!(
            "Updating Discord RPC: title='{}', artist='{}', elapsed={:?}, is_paused={}",
            track.title,
            track.artist,
            elapsed,
            is_paused
        );

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

        if let Err(e) = client.set_activity(act) {
            log::error!("Failed to set Discord RPC activity: {}", e);
        } else {
            log::debug!("Discord RPC activity updated successfully");
        }
    }

    pub fn clear(&mut self) {
        if let Some(ref mut client) = self.client {
            log::debug!("Clearing Discord RPC activity");
            if let Err(e) = client.clear_activity() {
                log::error!("Failed to clear Discord RPC activity: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_connection() {
        if std::env::var("CI").is_ok() {
            println!("Skipping Discord RPC connection test on CI runner");
            return;
        }
        let config = Config::new();
        let is_enabled = config
            .get_discord_settings()
            .map(|s| s.rpc_enabled)
            .unwrap_or(true);
        if !is_enabled {
            println!("Skipping Discord RPC connection test because rpc_enabled is false in config");
            return;
        }
        let mut rpc = DiscordRpc::new(&config);
        rpc.connect("1089228496459345970")
            .expect("Failed to connect to Discord RPC socket!");
        assert!(rpc.client.is_some(), "Failed to connect to Discord RPC!");

        let track = TrackInfo {
            id: "dZzvLtTgzlQ".to_string(),
            title: "Test Song Title".to_string(),
            artist: "Test Artist".to_string(),
            duration_secs: Some(200),
        };
        rpc.update(&track, Duration::from_secs(5), false);
        log::info!("Sleeping for 10 seconds to keep activity alive...");
        std::thread::sleep(Duration::from_secs(10));
    }
}
