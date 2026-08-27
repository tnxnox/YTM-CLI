use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscordSettings {
    pub enabled: bool,
    pub token: String,
    pub channel_id: String,
    #[serde(default = "default_rpc_enabled")]
    pub rpc_enabled: bool,
    #[serde(default = "default_client_id")]
    pub client_id: String,
}

fn default_rpc_enabled() -> bool {
    true
}

fn default_client_id() -> String {
    "1089228496459345970".to_string()
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
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            let local_node = if cfg!(windows) {
                Path::new(&home).join(".local").join("bin").join("node.exe")
            } else {
                Path::new(&home).join(".local").join("bin").join("node")
            };
            if local_node.exists() {
                return format!("node:{}", local_node.to_string_lossy());
            }
        }
        "node".to_string()
    }

    pub fn resolve_browser_cookie_arg(&self, browser: &str) -> String {
        if browser.trim().eq_ignore_ascii_case("zen") {
            if let Some(profile_dir) = self.find_zen_profile_dir() {
                return format!("firefox:{}", profile_dir.display());
            }
            return "firefox".to_string();
        }
        browser.to_string()
    }

    pub fn get_cookie_browser_arg(&self) -> Option<String> {
        self.get_browser()
            .map(|b| self.resolve_browser_cookie_arg(&b))
    }

    fn find_zen_profile_dir(&self) -> Option<std::path::PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        let home_path = std::path::Path::new(&home);

        let candidate_parents = vec![
            home_path.join(".config").join("zen"),
            home_path.join(".zen"),
            home_path
                .join(".var")
                .join("app")
                .join("app.zen_browser.zen")
                .join("data")
                .join("zen"),
            home_path
                .join(".var")
                .join("app")
                .join("io.github.zen_browser.zen")
                .join("data")
                .join("zen"),
            home_path
                .join("Library")
                .join("Application Support")
                .join("zen"),
            home_path
                .join("Library")
                .join("Application Support")
                .join("Zen"),
            home_path
                .join("AppData")
                .join("Roaming")
                .join("zen")
                .join("Profiles"),
            home_path
                .join("AppData")
                .join("Roaming")
                .join("Zen")
                .join("Profiles"),
        ];

        for parent in candidate_parents {
            if !parent.exists() {
                continue;
            }

            if parent.join("cookies.sqlite").exists() {
                return Some(parent);
            }

            if let Ok(entries) = std::fs::read_dir(&parent) {
                let mut subdirs = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if path.join("cookies.sqlite").exists() {
                            return Some(path);
                        }
                        subdirs.push(path);
                    }
                }
                if let Some(default_dir) = subdirs.iter().find(|p| {
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    name.contains("default")
                }) {
                    return Some(default_dir.clone());
                }
                if let Some(first) = subdirs.first() {
                    return Some(first.clone());
                }
            }
        }
        None
    }

    pub async fn login(&self, browser: &str) -> Result<(), anyhow::Error> {
        let yt_dlp_path = self.ensure_yt_dlp().await?;
        let cookie_arg = self.resolve_browser_cookie_arg(browser);
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
                &cookie_arg,
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
            Err(anyhow::anyhow!(
                "Login failed. Make sure the browser '{}' is installed, closed (if Chromium-based), and you are logged into YouTube/YouTube Music on it.\nError details: {}",
                browser,
                stderr
            ))
        }
    }

    pub async fn ensure_yt_dlp(&self) -> Result<PathBuf, anyhow::Error> {
        let venv_dir = self.db_path.parent().unwrap().join("venv");

        let (bin_dir_name, yt_dlp_name, pip_name) = if cfg!(windows) {
            ("Scripts", "yt-dlp.exe", "pip.exe")
        } else {
            ("bin", "yt-dlp", "pip")
        };

        // 1. Check if we have a working yt-dlp in our local data directory venv
        let local_venv_bin = venv_dir.join(bin_dir_name).join(yt_dlp_name);
        if local_venv_bin.exists() {
            return Ok(local_venv_bin);
        }

        // 2. Check if we have a dev venv in the current directory
        let dev_venv_bin = PathBuf::from("venv").join(bin_dir_name).join(yt_dlp_name);
        if dev_venv_bin.exists() {
            return Ok(dev_venv_bin);
        }

        // 3. If neither exists, we'll auto-initialize a virtualenv in the local data directory and install yt-dlp
        println!("Initializing local yt-dlp python dependency (one-time setup)...");

        // Try commands sequentially: "python3", "python", then "py"
        let python_cmds = if cfg!(windows) {
            vec!["python", "py", "python3"]
        } else {
            vec!["python3", "python"]
        };

        let mut success = false;
        for cmd in &python_cmds {
            let status = tokio::process::Command::new(cmd)
                .args(&["-m", "venv", &venv_dir.to_string_lossy()])
                .status()
                .await;

            if let Ok(s) = status {
                if s.success() {
                    success = true;
                    break;
                }
            }
        }

        if !success {
            return Err(anyhow::anyhow!(
                "Failed to create python virtual environment. Please ensure Python is installed and in your PATH (tried commands: {:?})",
                python_cmds
            ));
        }

        // Run pip install -U yt-dlp
        let pip_bin = venv_dir.join(bin_dir_name).join(pip_name);
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

    pub async fn update_yt_dlp(&self) -> Result<(), anyhow::Error> {
        let venv_dir = self.db_path.parent().unwrap().join("venv");
        let (bin_dir_name, pip_name) = if cfg!(windows) {
            ("Scripts", "pip.exe")
        } else {
            ("bin", "pip")
        };
        let pip_bin = venv_dir.join(bin_dir_name).join(pip_name);
        if pip_bin.exists() {
            println!("  🔄 Updating local yt-dlp dependency...");
            let status = tokio::process::Command::new(&pip_bin)
                .args(&["install", "-U", "yt-dlp"])
                .status()
                .await?;
            if status.success() {
                println!("  ✅ yt-dlp updated successfully!");
                return Ok(());
            }
        }
        Err(anyhow::anyhow!("Failed to update yt-dlp"))
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
