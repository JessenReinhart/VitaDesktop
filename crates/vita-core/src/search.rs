use crate::models::{AppCategory, InstalledApp};

/// Maps a Windows application display name + AUMID into a clean launcher category.
pub fn classify_app_category(name: &str, aumid: &str) -> AppCategory {
    let n = name.to_ascii_lowercase();
    let a = aumid.to_ascii_lowercase();

    let matches_any = |needles: &[&str]| {
        needles
            .iter()
            .any(|k| n.contains(k) || a.contains(k))
    };

    if matches_any(&[
        "visual studio",
        "vscode",
        "code",
        "terminal",
        "powershell",
        "command prompt",
        "cmd.exe",
        "git",
        "python",
        "node",
        "docker",
        "wsl",
        "rust",
        "intellij",
        "android studio",
        "winscp",
        "putty",
    ]) {
        AppCategory::Developer
    } else if matches_any(&[
        "edge",
        "chrome",
        "firefox",
        "brave",
        "opera",
        "mail",
        "outlook",
        "teams",
        "discord",
        "slack",
        "telegram",
        "whatsapp",
        "zoom",
        "browser",
        "store",
    ]) {
        AppCategory::Internet
    } else if matches_any(&[
        "photos",
        "paint",
        "figma",
        "blender",
        "gimp",
        "photoshop",
        "illustrator",
        "inkscape",
        "krita",
        "canva",
    ]) {
        AppCategory::Graphics
    } else if matches_any(&[
        "spotify",
        "media player",
        "vlc",
        "music",
        "sound",
        "video",
        "movies",
        "clipchamp",
        "obs",
        "audacity",
        "camera",
    ]) {
        AppCategory::Media
    } else if matches_any(&[
        "word",
        "excel",
        "powerpoint",
        "onenote",
        "notepad",
        "calendar",
        "todo",
        "to do",
        "journal",
        "adobe acrobat",
        "libreoffice",
        "notion",
        "obsidian",
    ]) {
        AppCategory::Office
    } else if matches_any(&[
        "calculator",
        "snipping",
        "clock",
        "sticky notes",
        "maps",
        "weather",
        "voice recorder",
        "sound recorder",
        "magnifier",
        "character map",
    ]) {
        AppCategory::Accessories
    } else if matches_any(&[
        "xbox",
        "steam",
        "epic games",
        "solitaire",
        "game",
        "minecraft",
    ]) {
        AppCategory::Games
    } else if matches_any(&[
        "settings",
        "control panel",
        "task manager",
        "registry",
        "file explorer",
        "explorer",
        "defender",
        "security",
        "device manager",
        "disk",
        "resource monitor",
        "event viewer",
        "services",
        "system information",
        "run",
        "windows tools",
        "backup",
    ]) {
        AppCategory::System
    } else {
        AppCategory::Other
    }
}

/// Fast deterministic fuzzy scorer prioritizing exact, prefix, word-boundary, substring, and subsequence hits.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let c = candidate.to_ascii_lowercase();
    if c == q {
        return Some(10_000);
    }
    if c.starts_with(&q) {
        return Some(8_000 - c.len() as i32);
    }

    // Word-boundary prefix match (e.g. "term" matches "Windows Terminal").
    for (idx, word) in c.split(|ch: char| !ch.is_alphanumeric()).enumerate() {
        if word.starts_with(&q) {
            return Some(6_500 - (idx as i32 * 40) - c.len() as i32);
        }
    }

    if let Some(pos) = c.find(&q) {
        return Some(4_500 - (pos as i32 * 10) - c.len() as i32);
    }

    // Ordered character subsequence match (e.g. "vsc" -> "Visual Studio Code").
    let q_chars: Vec<char> = q.chars().collect();
    let mut qi = 0usize;
    let mut score: i32 = 2_000;
    let mut prev_match_idx: Option<usize> = None;
    let mut prev_sep = true;

    for (ci, ch) in c.chars().enumerate() {
        if qi < q_chars.len() && ch == q_chars[qi] {
            if prev_sep {
                score += 60;
            }
            if let Some(prev) = prev_match_idx {
                let gap = ci - prev - 1;
                if gap == 0 {
                    score += 45;
                } else {
                    score -= (gap as i32) * 6;
                }
            } else {
                score -= (ci as i32) * 3;
            }
            prev_match_idx = Some(ci);
            qi += 1;
        }
        prev_sep = !ch.is_alphanumeric();
    }

    if qi == q_chars.len() {
        Some(score.max(1) - c.len() as i32)
    } else {
        None
    }
}

/// Filters installed apps by query (searching across all categories) or selected category.
pub fn filter_and_sort_apps<'a>(
    apps: &'a [InstalledApp],
    query: &str,
    category: AppCategory,
) -> Vec<&'a InstalledApp> {
    let trimmed = query.trim();
    let mut scored: Vec<(i32, &'a InstalledApp)> = apps
        .iter()
        .filter_map(|app| {
            if trimmed.is_empty() && category != AppCategory::All && app.category != category {
                return None;
            }
            let s = fuzzy_score(trimmed, &app.name)?;
            Some((s, app))
        })
        .collect();

    scored.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    scored.into_iter().map(|(_, app)| app).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(name: &str, aumid: &str) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            aumid: aumid.into(),
            category: classify_app_category(name, aumid),
            icon_cache_path: None,
            icon: None,
        }
    }

    #[test]
    fn fuzzy_ranks_exact_prefix_word_and_subsequence_properly() {
        let apps = vec![
            mk("Windows Terminal", "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"),
            mk("Calculator", "Microsoft.WindowsCalculator!App"),
            mk("Visual Studio Code", "Microsoft.VisualStudioCode"),
            mk("Character Map", "charmap.exe"),
        ];

        let term_hits = filter_and_sort_apps(&apps, "term", AppCategory::All);
        assert_eq!(term_hits.first().unwrap().name, "Windows Terminal");

        let calc_hits = filter_and_sort_apps(&apps, "calc", AppCategory::All);
        assert_eq!(calc_hits.first().unwrap().name, "Calculator");

        let vsc_hits = filter_and_sort_apps(&apps, "vsc", AppCategory::All);
        assert_eq!(vsc_hits.first().unwrap().name, "Visual Studio Code");
    }

    #[test]
    fn category_classification_and_filtering_work() {
        let apps = vec![
            mk("Terminal", "wt.exe"),
            mk("Microsoft Edge", "MSEdge"),
            mk("Settings", "ms-settings:"),
            mk("Calculator", "calc.exe"),
        ];

        let dev = filter_and_sort_apps(&apps, "", AppCategory::Developer);
        assert_eq!(dev.len(), 1);
        assert_eq!(dev[0].name, "Terminal");

        // Non-empty query searches across all categories regardless of selected category tab.
        let cross = filter_and_sort_apps(&apps, "calc", AppCategory::Developer);
        assert_eq!(cross.len(), 1);
        assert_eq!(cross[0].name, "Calculator");
    }
}
