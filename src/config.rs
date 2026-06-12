use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscordSettings {
    pub enabled: bool,
    pub token: String,
    pub channel_id: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub cache_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cookies_path: PathBuf,
    pub discord_settings_path: PathBuf,
}

impl Config {
    pub fn new() -> Self {
        let proj_dirs = ProjectDirs::from("com", "ytm-cli", "ytm-cli")
            .expect("Failed to determine project directories");

        let db_path = proj_dirs.data_dir().join("db.sqlite");
        let cache_dir = proj_dirs.data_dir().join("cache");
        let config_dir = proj_dirs.config_dir().to_path_buf();
        let cookies_path = config_dir.join("cookies.txt");
        let discord_settings_path = config_dir.join("discord.json");

        // Ensure directories exist
        std::fs::create_dir_all(proj_dirs.data_dir()).ok();
        std::fs::create_dir_all(&cache_dir).ok();
        std::fs::create_dir_all(&config_dir).ok();

        Self {
            db_path,
            cache_dir,
            config_dir,
            cookies_path,
            discord_settings_path,
        }
    }

    pub fn is_logged_in(&self) -> bool {
        let browser_path = self.config_dir.join("browser.txt");
        if browser_path.exists() {
            true
        } else {
            self.cookies_path.exists()
                && std::fs::metadata(&self.cookies_path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
        }
    }

    pub fn logout(&self) -> std::io::Result<()> {
        let browser_path = self.config_dir.join("browser.txt");
        if browser_path.exists() {
            std::fs::remove_file(&browser_path)?;
        }
        if self.cookies_path.exists() {
            std::fs::remove_file(&self.cookies_path)?;
        }
        Ok(())
    }

    pub fn get_browser(&self) -> Option<String> {
        let browser_path = self.config_dir.join("browser.txt");
        if browser_path.exists() {
            std::fs::read_to_string(&browser_path)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    }

    pub fn get_js_runtime_arg(&self) -> String {
        use std::path::Path;
        if let Ok(home) = std::env::var("HOME") {
            let local_node = Path::new(&home).join(".local").join("bin").join("node");
            if local_node.exists() {
                return format!("node:{}", local_node.to_string_lossy());
            }
        }
        "node".to_string()
    }

    pub async fn login(&self, browser: &str) -> Result<(), anyhow::Error> {
        let yt_dlp_path = self.ensure_yt_dlp().await?;
        println!(
            "  🔑 Verifying cookies from {}... (Please close the browser if it is Chromium-based)",
            browser
        );

        let output = tokio::process::Command::new(&yt_dlp_path)
            .args(&[
                "--js-runtimes",
                &self.get_js_runtime_arg(),
                "--remote-components",
                "ejs:github",
                "--cookies-from-browser",
                browser,
                "--skip-download",
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            ])
            .output()
            .await?;

        if output.status.success() {
            let browser_path = self.config_dir.join("browser.txt");
            std::fs::write(&browser_path, browser)?;
            // Clean up old cookies.txt to avoid confusion
            if self.cookies_path.exists() {
                let _ = std::fs::remove_file(&self.cookies_path);
            }
            println!("  ✅ Login successful!");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow::anyhow!("Login failed. Make sure the browser '{}' is installed, closed (if Chromium-based), and you are logged into YouTube/YouTube Music on it.\nError details: {}", browser, stderr))
        }
    }

    pub async fn ensure_yt_dlp(&self) -> Result<PathBuf, anyhow::Error> {
        // 1. Check if we have a working yt-dlp in our local data directory venv
        let local_venv_bin = self
            .db_path
            .parent()
            .unwrap()
            .join("venv")
            .join("bin")
            .join("yt-dlp");

        if local_venv_bin.exists() {
            return Ok(local_venv_bin);
        }

        // 2. Check if we have a dev venv in the current directory
        let dev_venv_bin = PathBuf::from("venv").join("bin").join("yt-dlp");
        if dev_venv_bin.exists() {
            return Ok(dev_venv_bin);
        }

        // 3. If neither exists, we'll auto-initialize a virtualenv in the local data directory and install yt-dlp
        let venv_dir = self.db_path.parent().unwrap().join("venv");
        println!("Initializing local yt-dlp python dependency (one-time setup)...");

        // Run python3 -m venv <venv_dir>
        let status = tokio::process::Command::new("python3")
            .args(&["-m", "venv", &venv_dir.to_string_lossy()])
            .status()
            .await?;

        if !status.success() {
            return Err(anyhow::anyhow!(
                "Failed to create python virtual environment at {:?}",
                venv_dir
            ));
        }

        // Run venv/bin/pip install -U yt-dlp
        let pip_bin = venv_dir.join("bin").join("pip");
        let status = tokio::process::Command::new(&pip_bin)
            .args(&["install", "-U", "yt-dlp"])
            .status()
            .await?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to install yt-dlp via pip"));
        }

        if local_venv_bin.exists() {
            println!("yt-dlp initialized successfully!");
            Ok(local_venv_bin)
        } else {
            Err(anyhow::anyhow!(
                "yt-dlp executable not found after successful installation"
            ))
        }
    }

    pub fn get_discord_settings(&self) -> Option<DiscordSettings> {
        if self.discord_settings_path.exists() {
            let content = std::fs::read_to_string(&self.discord_settings_path).ok()?;
            serde_json::from_str(&content).ok()
        } else {
            None
        }
    }

    pub fn save_discord_settings(&self, settings: &DiscordSettings) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(settings).unwrap();
        std::fs::write(&self.discord_settings_path, content)?;
        Ok(())
    }
}
