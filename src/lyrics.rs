use anyhow::Result;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SyncedWord {
    pub start_time: Duration,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct SyncedLine {
    pub start_time: Duration,
    pub end_time: Option<Duration>,
    pub text: String,
    pub words: Vec<SyncedWord>,
}

#[derive(Debug, Clone)]
pub struct LyricsData {
    pub lines: Vec<SyncedLine>,
}

#[derive(Debug, Deserialize)]
struct LrclibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
}

pub fn parse_timestamp(s: &str) -> Option<Duration> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let minutes: u64 = parts[0].trim().parse().ok()?;
    let rest = parts[1].trim();

    let (seconds_str, ms_str) = if let Some(dot_idx) = rest.find('.') {
        (&rest[..dot_idx], &rest[dot_idx + 1..])
    } else {
        (rest, "")
    };

    let seconds: u64 = seconds_str.parse().ok()?;
    let mut ms: u64 = 0;
    if !ms_str.is_empty() {
        let val: u64 = ms_str.parse().ok()?;
        if ms_str.len() == 1 {
            ms = val * 100;
        } else if ms_str.len() == 2 {
            ms = val * 10;
        } else if ms_str.len() == 3 {
            ms = val;
        } else {
            let mut s_str = ms_str.to_string();
            s_str.truncate(3);
            ms = s_str.parse().unwrap_or(0);
        }
    }

    Some(Duration::from_millis(
        minutes * 60 * 1000 + seconds * 1000 + ms,
    ))
}

pub fn parse_lrc_line(line_str: &str) -> Option<SyncedLine> {
    let trimmed = line_str.trim();
    if !trimmed.starts_with('[') {
        return None;
    }

    let close_bracket = trimmed.find(']')?;
    let time_str = &trimmed[1..close_bracket];
    let start_time = parse_timestamp(time_str)?;

    let remainder = trimmed[close_bracket + 1..].trim();
    if remainder.is_empty() {
        return Some(SyncedLine {
            start_time,
            end_time: None,
            text: String::new(),
            words: Vec::new(),
        });
    }

    let mut words = Vec::new();
    let mut full_text = String::new();

    if remainder.contains('<') && remainder.contains('>') {
        let mut curr_pos = 0;
        while let Some(start_tag) = remainder[curr_pos..].find('<') {
            let tag_start = curr_pos + start_tag;
            if let Some(end_tag) = remainder[tag_start..].find('>') {
                let tag_end = tag_start + end_tag;
                let word_time_str = &remainder[tag_start + 1..tag_end];

                let word_text_start = tag_end + 1;
                let next_tag = remainder[word_text_start..]
                    .find('<')
                    .map(|p| word_text_start + p)
                    .unwrap_or(remainder.len());
                let word_text = &remainder[word_text_start..next_tag];

                if let Some(w_time) = parse_timestamp(word_time_str) {
                    words.push(SyncedWord {
                        start_time: w_time,
                        text: word_text.to_string(),
                    });
                    full_text.push_str(word_text);
                }
                curr_pos = next_tag;
            } else {
                break;
            }
        }
    }

    if words.is_empty() {
        full_text = remainder.to_string();
    }

    Some(SyncedLine {
        start_time,
        end_time: None,
        text: full_text,
        words,
    })
}

pub fn parse_lrc(lrc_text: &str) -> Option<LyricsData> {
    let mut lines = Vec::new();
    for line in lrc_text.lines() {
        if let Some(parsed) = parse_lrc_line(line) {
            lines.push(parsed);
        }
    }
    if lines.is_empty() {
        return None;
    }

    lines.sort_by_key(|l| l.start_time);

    for i in 0..lines.len() {
        let char_count = lines[i].text.chars().count();
        let vocal_est = Duration::from_millis((char_count as u64 * 180).clamp(2000, 6000));

        if i + 1 < lines.len() {
            let gap = lines[i + 1].start_time.saturating_sub(lines[i].start_time);
            if gap <= Duration::from_millis(6000) {
                lines[i].end_time = Some(
                    lines[i + 1]
                        .start_time
                        .saturating_sub(Duration::from_millis(300)),
                );
            } else {
                lines[i].end_time = Some(lines[i].start_time + vocal_est);
            }
        } else {
            lines[i].end_time = Some(lines[i].start_time + vocal_est);
        }
    }

    Some(LyricsData { lines })
}

use std::collections::HashMap;
use std::sync::Mutex;

static LYRICS_CACHE: std::sync::LazyLock<Mutex<HashMap<String, Option<LyricsData>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn get_cache_key(title: &str, artist: &str) -> String {
    format!(
        "{}|||{}",
        title.trim().to_lowercase(),
        artist.trim().to_lowercase()
    )
}

pub fn render_active_line(
    line: &SyncedLine,
    elapsed: Duration,
    visualizer: Option<&crate::audio::VisualizerShared>,
) -> String {
    if line.text.is_empty() {
        return String::new();
    }

    // Lead offset (+100ms): Make words light up at the START of being sung
    let adjusted_elapsed = elapsed + Duration::from_millis(100);

    // Word-level highlighting (Enhanced LRC with explicit timestamps)
    if !line.words.is_empty() {
        let mut result = String::new();
        for word in &line.words {
            if adjusted_elapsed >= word.start_time {
                result.push_str(&crate::theme::style_accent(&word.text).to_string());
            } else {
                result.push_str(&crate::theme::style_dim(&word.text).to_string());
            }
        }
        return result;
    }

    // Character-weighted word progress for Standard Line LRC
    let start = line.start_time;
    let raw_gap = line
        .end_time
        .unwrap_or(start + Duration::from_secs(4))
        .saturating_sub(start);

    if adjusted_elapsed < start {
        return crate::theme::style_dim(&line.text).to_string();
    }

    // Vocals usually span most of the line gap (leaving a short ~350ms breath pause before next line).
    // For long instrumental breaks (> 6.5s), cap the active line duration based on character length.
    let sing_duration = if raw_gap.as_secs_f32() <= 6.5 {
        raw_gap
            .saturating_sub(Duration::from_millis(350))
            .max(Duration::from_millis(1000))
    } else {
        let char_count = line.text.chars().count();
        let max_sing_ms = (char_count as u64 * 200).clamp(3000, 6500);
        Duration::from_millis(max_sing_ms.min(raw_gap.as_millis() as u64))
    };

    let end = start + sing_duration;
    if adjusted_elapsed >= end {
        return crate::theme::style_accent(&line.text).to_string();
    }

    let duration_ms = sing_duration.as_millis() as f32;
    let elapsed_in_line = adjusted_elapsed.saturating_sub(start).as_millis() as f32;
    let time_fraction = (elapsed_in_line / duration_ms).clamp(0.0, 1.0);

    // Hybrid Approach: Sample vocal energy from FFT midrange bands (bands 2, 3, 4)
    let vocal_energy = if let Some(vis) = visualizer {
        let b2 = vis.get_band(2);
        let b3 = vis.get_band(3);
        let b4 = vis.get_band(4);
        ((b2 + b3 * 1.5 + b4) / 3.5).clamp(0.0, 1.0)
    } else {
        0.5
    };

    // Vocal Gating:
    // When active vocal energy is present (> 0.08), progress advances smoothly.
    // When vocal energy drops (pause/breath), progress holds steady on current word!
    let progress_fraction = if vocal_energy < 0.08 {
        (time_fraction * 0.85).clamp(0.0, 1.0)
    } else {
        (time_fraction + (vocal_energy - 0.08) * 0.15).clamp(0.0, 1.0)
    };

    let words: Vec<&str> = line.text.split_inclusive(' ').collect();
    if words.is_empty() {
        return crate::theme::style_accent(&line.text).to_string();
    }

    let total_chars: usize = words.iter().map(|w| w.chars().count()).sum();
    if total_chars == 0 {
        return crate::theme::style_accent(&line.text).to_string();
    }

    let target_char_count = (progress_fraction * total_chars as f32).round() as usize;

    let mut current_chars = 0;
    let mut highlighted_word_count = 0;

    for word in &words {
        let w_len = word.chars().count();
        if current_chars + w_len / 2 < target_char_count || highlighted_word_count == 0 {
            current_chars += w_len;
            highlighted_word_count += 1;
        } else {
            break;
        }
    }

    highlighted_word_count = highlighted_word_count.min(words.len());

    let sung_part: String = words.iter().take(highlighted_word_count).copied().collect();
    let unsung_part: String = words.iter().skip(highlighted_word_count).copied().collect();

    format!(
        "{}{}",
        crate::theme::style_accent(&sung_part),
        crate::theme::style_dim(&unsung_part)
    )
}

pub async fn fetch_lyrics(
    title: &str,
    artist: &str,
    duration_secs: Option<u32>,
) -> Result<Option<LyricsData>> {
    let cache_key = get_cache_key(title, artist);
    if let Ok(guard) = LYRICS_CACHE.lock() {
        if let Some(cached) = guard.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    let fetch_future = async {
        let client = reqwest::Client::builder()
            .user_agent("YTM-CLI/1.6.0 (https://github.com/tnxnox/YTM-CLI)")
            .timeout(Duration::from_millis(2000))
            .connect_timeout(Duration::from_millis(1500))
            .build()?;

        let mut params = vec![
            ("track_name", title.to_string()),
            ("artist_name", artist.to_string()),
        ];
        if let Some(dur) = duration_secs {
            params.push(("duration", dur.to_string()));
        }

        let mut result_data = None;

        if let Ok(res) = client
            .get("https://lrclib.net/api/get")
            .query(&params)
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(data) = res.json::<LrclibResponse>().await {
                    if let Some(lrc_text) = data.synced_lyrics {
                        if let Some(parsed) = parse_lrc(&lrc_text) {
                            result_data = Some(parsed);
                        }
                    }
                }
            }
        }

        if result_data.is_none() {
            let query = format!("{} {}", title, artist);
            if let Ok(res) = client
                .get("https://lrclib.net/api/search")
                .query(&[("q", &query)])
                .send()
                .await
            {
                if res.status().is_success() {
                    if let Ok(results) = res.json::<Vec<LrclibResponse>>().await {
                        for item in results {
                            if let Some(lrc_text) = item.synced_lyrics {
                                if let Some(parsed) = parse_lrc(&lrc_text) {
                                    result_data = Some(parsed);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok::<Option<LyricsData>, anyhow::Error>(result_data)
    };

    let result = match tokio::time::timeout(Duration::from_millis(3000), fetch_future).await {
        Ok(Ok(data)) => data,
        _ => None,
    };

    if let Ok(mut guard) = LYRICS_CACHE.lock() {
        guard.insert(cache_key, result.clone());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        assert_eq!(
            parse_timestamp("01:23.45"),
            Some(Duration::from_millis(83450))
        );
        assert_eq!(
            parse_timestamp("00:05.100"),
            Some(Duration::from_millis(5100))
        );
        assert_eq!(parse_timestamp("02:10"), Some(Duration::from_secs(130)));
    }

    #[test]
    fn test_parse_lrc_line() {
        let line = "[00:12.50]Hello world";
        let parsed = parse_lrc_line(line).unwrap();
        assert_eq!(parsed.start_time, Duration::from_millis(12500));
        assert_eq!(parsed.text, "Hello world");
        assert!(parsed.words.is_empty());
    }

    #[test]
    fn test_parse_enhanced_lrc_line() {
        let line = "[00:10.00]<00:10.00>Hello <00:11.50>world";
        let parsed = parse_lrc_line(line).unwrap();
        assert_eq!(parsed.start_time, Duration::from_millis(10000));
        assert_eq!(parsed.text, "Hello world");
        assert_eq!(parsed.words.len(), 2);
        assert_eq!(parsed.words[0].start_time, Duration::from_millis(10000));
        assert_eq!(parsed.words[0].text, "Hello ");
        assert_eq!(parsed.words[1].start_time, Duration::from_millis(11500));
        assert_eq!(parsed.words[1].text, "world");
    }
}
