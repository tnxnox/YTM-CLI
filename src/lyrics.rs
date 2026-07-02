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
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

pub fn parse_plain_lyrics(plain_text: &str, duration_secs: Option<u32>) -> Option<LyricsData> {
    let raw_lines: Vec<&str> = plain_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if raw_lines.is_empty() {
        return None;
    }

    let total_secs = duration_secs.unwrap_or(180).max(30) as f64;
    let step_secs = total_secs / raw_lines.len() as f64;

    let mut lines = Vec::new();
    for (i, text) in raw_lines.into_iter().enumerate() {
        let start_ms = (i as f64 * step_secs * 1000.0) as u64;
        let end_ms = (((i + 1) as f64 * step_secs * 1000.0) as u64).saturating_sub(300);
        lines.push(SyncedLine {
            start_time: Duration::from_millis(start_ms),
            end_time: Some(Duration::from_millis(end_ms)),
            text: text.to_string(),
            words: Vec::new(),
        });
    }

    Some(LyricsData { lines })
}

fn try_parse_response(data: &LrclibResponse, duration_secs: Option<u32>) -> Option<LyricsData> {
    if let Some(ref lrc_text) = data.synced_lyrics {
        if !lrc_text.trim().is_empty() {
            if let Some(parsed) = parse_lrc(lrc_text) {
                return Some(parsed);
            }
        }
    }

    if data.instrumental {
        return Some(LyricsData {
            lines: vec![SyncedLine {
                start_time: Duration::from_secs(0),
                end_time: Some(Duration::from_secs(duration_secs.unwrap_or(180) as u64)),
                text: "🎵 (Instrumental)".to_string(),
                words: Vec::new(),
            }],
        });
    }

    if let Some(ref plain_text) = data.plain_lyrics {
        if !plain_text.trim().is_empty() {
            if let Some(parsed) = parse_plain_lyrics(plain_text, duration_secs) {
                return Some(parsed);
            }
        }
    }

    None
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
    max_width: usize,
) -> String {
    if line.text.is_empty() {
        return String::new();
    }

    // Lead offset (+100ms): Make words light up at the START of being sung
    let adjusted_elapsed = elapsed + Duration::from_millis(100);

    // Word-level highlighting (Enhanced LRC with explicit timestamps)
    if !line.words.is_empty() {
        let mut result = String::new();
        let mut char_budget = max_width;
        for word in &line.words {
            let w_chars = word.text.chars().count();
            if char_budget == 0 {
                break;
            }
            let word_text = if w_chars > char_budget {
                let mut tr: String = word
                    .text
                    .chars()
                    .take(char_budget.saturating_sub(1))
                    .collect();
                tr.push('…');
                char_budget = 0;
                tr
            } else {
                char_budget = char_budget.saturating_sub(w_chars);
                word.text.clone()
            };

            if adjusted_elapsed >= word.start_time {
                result.push_str(&crate::theme::style_accent(&word_text).to_string());
            } else {
                result.push_str(&crate::theme::style_dim(&word_text).to_string());
            }
        }
        return result;
    }

    let text_to_render = if line.text.chars().count() > max_width {
        let mut tr: String = line
            .text
            .chars()
            .take(max_width.saturating_sub(1))
            .collect();
        tr.push('…');
        tr
    } else {
        line.text.clone()
    };

    // Character-weighted word progress for Standard Line LRC
    let start = line.start_time;
    let raw_gap = line
        .end_time
        .unwrap_or(start + Duration::from_secs(4))
        .saturating_sub(start);

    if adjusted_elapsed < start {
        return crate::theme::style_dim(&text_to_render).to_string();
    }

    // Vocals usually span most of the line gap (leaving a short ~350ms breath pause before next line).
    // For long instrumental breaks (> 6.5s), cap the active line duration based on character length.
    let sing_duration = if raw_gap.as_secs_f32() <= 6.5 {
        raw_gap
            .saturating_sub(Duration::from_millis(350))
            .max(Duration::from_millis(1000))
    } else {
        let char_count = text_to_render.chars().count();
        let max_sing_ms = (char_count as u64 * 200).clamp(3000, 6500);
        Duration::from_millis(max_sing_ms.min(raw_gap.as_millis() as u64))
    };

    let end = start + sing_duration;
    if adjusted_elapsed >= end {
        return crate::theme::style_accent(&text_to_render).to_string();
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

    let words: Vec<&str> = text_to_render.split_inclusive(' ').collect();
    if words.is_empty() {
        return crate::theme::style_accent(&text_to_render).to_string();
    }

    let total_chars: usize = words.iter().map(|w| w.chars().count()).sum();
    if total_chars == 0 {
        return crate::theme::style_accent(&text_to_render).to_string();
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

static BLOCKING_HTTP_CLIENT: std::sync::LazyLock<reqwest::blocking::Client> =
    std::sync::LazyLock::new(|| {
        reqwest::blocking::Client::builder()
            .user_agent("YTM-CLI/1.6.1 (https://github.com/tnxnox/YTM-CLI)")
            .timeout(Duration::from_millis(8000))
            .connect_timeout(Duration::from_millis(5000))
            .build()
            .unwrap_or_default()
    });

fn get_from_cache(key: &str) -> Option<Option<LyricsData>> {
    let guard = LYRICS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.get(key).cloned()
}

fn save_to_cache(key: String, data: Option<LyricsData>) {
    let mut guard = LYRICS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(key, data);
}

pub fn clean_track_title(title: &str) -> String {
    let mut filtered = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    filtered.push(c);
                }
            }
        }
    }

    let mut clean = filtered.trim();

    if let Some(dash_idx) = clean.find(" - ") {
        let suffix = clean[dash_idx + 3..].to_lowercase();
        if suffix.contains("official")
            || suffix.contains("video")
            || suffix.contains("audio")
            || suffix.contains("remaster")
            || suffix.contains("live")
            || suffix.contains("topic")
            || suffix.contains("version")
            || suffix.contains("hd")
            || suffix.contains("4k")
        {
            clean = clean[..dash_idx].trim();
        }
    }

    if clean.is_empty() {
        title.trim().to_string()
    } else {
        clean.to_string()
    }
}

pub fn clean_artist_name(artist: &str) -> String {
    let mut clean = artist.trim();
    if clean.to_lowercase().ends_with(" - topic") {
        clean = clean[..clean.len() - 8].trim();
    }
    if let Some(comma_idx) = clean.find(',') {
        clean = clean[..comma_idx].trim();
    }
    if clean.is_empty() {
        artist.trim().to_string()
    } else {
        clean.to_string()
    }
}

fn try_lrclib_get(title: &str, artist: &str, duration_secs: Option<u32>) -> Option<LyricsData> {
    let mut params = vec![
        ("track_name", title.to_string()),
        ("artist_name", artist.to_string()),
    ];
    if let Some(dur) = duration_secs {
        params.push(("duration", dur.to_string()));
    }

    for attempt in 0..2 {
        if let Ok(res) = BLOCKING_HTTP_CLIENT
            .get("https://lrclib.net/api/get")
            .query(&params)
            .send()
        {
            if res.status().is_success() {
                if let Ok(data) = res.json::<LrclibResponse>() {
                    if let Some(parsed) = try_parse_response(&data, duration_secs) {
                        return Some(parsed);
                    }
                }
            } else {
                break;
            }
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    None
}

fn try_lrclib_search(query: &str, duration_secs: Option<u32>) -> Option<LyricsData> {
    for attempt in 0..2 {
        if let Ok(res) = BLOCKING_HTTP_CLIENT
            .get("https://lrclib.net/api/search")
            .query(&[("q", query)])
            .send()
        {
            if res.status().is_success() {
                if let Ok(results) = res.json::<Vec<LrclibResponse>>() {
                    for item in results {
                        if let Some(parsed) = try_parse_response(&item, duration_secs) {
                            return Some(parsed);
                        }
                    }
                }
            } else {
                break;
            }
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    None
}

pub fn fetch_lyrics_blocking(
    title: &str,
    artist: &str,
    duration_secs: Option<u32>,
) -> Option<LyricsData> {
    let cache_key = get_cache_key(title, artist);
    if let Some(cached) = get_from_cache(&cache_key) {
        return cached;
    }

    let clean_t = clean_track_title(title);
    let clean_a = clean_artist_name(artist);
    let clean_key = get_cache_key(&clean_t, &clean_a);

    let save = |d: Option<LyricsData>| {
        save_to_cache(cache_key.clone(), d.clone());
        if clean_key != cache_key {
            save_to_cache(clean_key, d.clone());
        }
        d
    };

    // Tier 1: Exact match on cleaned title + artist + duration
    if let Some(data) = try_lrclib_get(&clean_t, &clean_a, duration_secs) {
        return save(Some(data));
    }

    // Tier 2: Exact match on cleaned title + artist (without duration constraint)
    if let Some(data) = try_lrclib_get(&clean_t, &clean_a, None) {
        return save(Some(data));
    }

    // Tier 3: Exact match on raw title + artist (without duration)
    if clean_t != title || clean_a != artist {
        if let Some(data) = try_lrclib_get(title, artist, None) {
            return save(Some(data));
        }
    }

    // Tier 4: Search query with cleaned title + artist
    let search_q = format!("{} {}", clean_t, clean_a);
    if let Some(data) = try_lrclib_search(&search_q, duration_secs) {
        return save(Some(data));
    }

    // Tier 5: Search query with raw title + artist
    if clean_t != title || clean_a != artist {
        let raw_q = format!("{} {}", title, artist);
        if let Some(data) = try_lrclib_search(&raw_q, duration_secs) {
            return save(Some(data));
        }
    }

    save(None)
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

    #[test]
    fn test_render_active_line_truncation() {
        let line = SyncedLine {
            start_time: Duration::from_secs(0),
            end_time: Some(Duration::from_secs(5)),
            text: "This is a very long lyric line that exceeds terminal width".to_string(),
            words: Vec::new(),
        };
        let rendered = render_active_line(&line, Duration::from_secs(1), None, 20);
        assert!(rendered.contains('…'));
    }

    #[test]
    fn test_clean_track_title() {
        assert_eq!(
            clean_track_title("Blinding Lights (Official Music Video)"),
            "Blinding Lights"
        );
        assert_eq!(clean_track_title("Starboy (feat. Daft Punk)"), "Starboy");
        assert_eq!(
            clean_track_title("Hotel California - 2013 Remaster"),
            "Hotel California"
        );
        assert_eq!(clean_track_title("Numb [Explicit]"), "Numb");
    }

    #[test]
    fn test_clean_artist_name() {
        assert_eq!(clean_artist_name("The Weeknd - Topic"), "The Weeknd");
        assert_eq!(clean_artist_name("Drake, Future"), "Drake");
    }

    #[test]
    fn test_fetch_lyrics_blocking_real() {
        let res = fetch_lyrics_blocking("FE!N", "Travis Scott", None);
        assert!(res.is_some(), "Expected lyrics for FE!N by Travis Scott");
    }
}
