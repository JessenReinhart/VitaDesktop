use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Single cell in the fixed 6x7 (42-cell) monthly calendar grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDay {
    pub date: NaiveDate,
    pub in_current_month: bool,
    pub is_today: bool,
    pub is_selected: bool,
    pub event_count: usize,
}

/// Persistent user reminder/event associated with a `YYYY-MM-DD` calendar date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: u64,
    pub date: String,
    pub title: String,
    #[serde(default)]
    pub done: bool,
}

/// Persistent store for calendar preferences and reminders (`%APPDATA%\VitaDesktop\calendar.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CalendarStore {
    pub monday_first: bool,
    pub events: Vec<CalendarEvent>,
    pub next_id: u64,
}

fn format_date_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

impl CalendarStore {
    /// Returns `%APPDATA%\VitaDesktop\calendar.toml` (falling back to `./calendar.toml`).
    pub fn store_path() -> PathBuf {
        crate::config::ShellConfig::config_dir().join("calendar.toml")
    }

    /// Parses a TOML string into a `CalendarStore`.
    pub fn from_toml_str(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
    }

    /// Parses a TOML string into a `CalendarStore`, returning `Default` if malformed.
    pub fn from_toml_or_default(contents: &str) -> Self {
        Self::from_toml_str(contents).unwrap_or_default()
    }

    /// Loads the calendar store from disk, or returns `Default` when missing or malformed.
    pub fn load_or_default() -> Self {
        Self::load_or_default_from(&Self::store_path())
    }

    /// Loads the calendar store from an explicit path, falling back to `Default` if missing or malformed.
    pub fn load_or_default_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => match Self::from_toml_str(&raw) {
                Ok(store) => store,
                Err(err) => {
                    tracing::warn!(
                        ?err,
                        path = %path.display(),
                        "Malformed calendar.toml, falling back to default"
                    );
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Atomically writes the calendar store to `%APPDATA%\VitaDesktop\calendar.toml`.
    pub fn save_atomic(&self) -> io::Result<()> {
        self.save_atomic_to(&Self::store_path())
    }

    /// Atomically writes the calendar store to an explicit path using a temporary `.toml.tmp` file.
    pub fn save_atomic_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    /// Adds a trimmed reminder on `date` and returns its unique ID, or `None` if `title` is blank.
    pub fn add_event(&mut self, date: NaiveDate, title: &str) -> Option<u64> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return None;
        }
        let max_existing = self.events.iter().map(|e| e.id).max().unwrap_or(0);
        let id = self.next_id.max(max_existing.saturating_add(1)).max(1);
        self.next_id = id.saturating_add(1);
        self.events.push(CalendarEvent {
            id,
            date: format_date_key(date),
            title: trimmed.to_string(),
            done: false,
        });
        Some(id)
    }

    /// Toggles the `done` state of the event with `id`.
    pub fn toggle_event(&mut self, id: u64) {
        if let Some(event) = self.events.iter_mut().find(|e| e.id == id) {
            event.done = !event.done;
        }
    }

    /// Removes the event with `id` if present.
    pub fn delete_event(&mut self, id: u64) {
        self.events.retain(|e| e.id != id);
    }

    /// Returns all events matching `date` in stored order.
    pub fn events_for_date(&self, date: NaiveDate) -> Vec<&CalendarEvent> {
        let key = format_date_key(date);
        self.events.iter().filter(|e| e.date == key).collect()
    }
}

/// Returns `(year, month)` for the month preceding `(year, month)`, wrapping Jan -> Dec.
pub fn prev_month(year: i32, month: u32) -> (i32, u32) {
    let m = month.clamp(1, 12);
    if m == 1 { (year - 1, 12) } else { (year, m - 1) }
}

/// Returns `(year, month)` for the month following `(year, month)`, wrapping Dec -> Jan.
pub fn next_month(year: i32, month: u32) -> (i32, u32) {
    let m = month.clamp(1, 12);
    if m == 12 { (year + 1, 1) } else { (year, m + 1) }
}

/// Builds a fixed 42-cell (6 weeks x 7 days) grid for `(year, month)` including leading and trailing dates.
pub fn build_month_grid(
    year: i32,
    month: u32,
    today: NaiveDate,
    selected: NaiveDate,
    monday_first: bool,
    events: &[CalendarEvent],
) -> Vec<CalendarDay> {
    let month = month.clamp(1, 12);
    let first_of_month = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today);
    let leading = if monday_first {
        first_of_month.weekday().num_days_from_monday()
    } else {
        first_of_month.weekday().num_days_from_sunday()
    };
    let start_date = first_of_month
        .checked_sub_days(Days::new(u64::from(leading)))
        .unwrap_or(first_of_month);

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for ev in events {
        *counts.entry(ev.date.as_str()).or_insert(0) += 1;
    }

    let mut days = Vec::with_capacity(42);
    for offset in 0..42_u64 {
        let date = start_date
            .checked_add_days(Days::new(offset))
            .unwrap_or(start_date);
        let event_count = if counts.is_empty() {
            0
        } else {
            let key = format_date_key(date);
            counts.get(key.as_str()).copied().unwrap_or(0)
        };
        days.push(CalendarDay {
            date,
            in_current_month: date.year() == year && date.month() == month,
            is_today: date == today,
            is_selected: date == selected,
            event_count,
        });
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn d(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    #[test]
    fn leap_year_feb_2024_vs_2025() {
        let today = d(2024, 2, 15);
        let feb_2024 = build_month_grid(2024, 2, today, today, true, &[]);
        let in_month_2024: Vec<NaiveDate> = feb_2024
            .iter()
            .filter(|c| c.in_current_month)
            .map(|c| c.date)
            .collect();
        assert_eq!(in_month_2024.len(), 29);
        assert_eq!(in_month_2024.last().copied(), Some(d(2024, 2, 29)));

        let feb_2025 = build_month_grid(2025, 2, d(2025, 2, 15), d(2025, 2, 15), true, &[]);
        let in_month_2025: Vec<NaiveDate> = feb_2025
            .iter()
            .filter(|c| c.in_current_month)
            .map(|c| c.date)
            .collect();
        assert_eq!(in_month_2025.len(), 28);
        assert_eq!(in_month_2025.last().copied(), Some(d(2025, 2, 28)));
    }

    #[test]
    fn month_navigation_wraps_across_years() {
        assert_eq!(prev_month(2026, 1), (2025, 12));
        assert_eq!(prev_month(2026, 11), (2026, 10));
        assert_eq!(next_month(2025, 12), (2026, 1));
        assert_eq!(next_month(2026, 11), (2026, 12));
    }

    #[test]
    fn always_42_cells_across_all_12_months() {
        for year in [2024, 2025, 2026] {
            for month in 1..=12 {
                for monday_first in [false, true] {
                    let today = d(year, month, 1);
                    let grid = build_month_grid(year, month, today, today, monday_first, &[]);
                    assert_eq!(
                        grid.len(),
                        42,
                        "Expected 42 cells for {year}-{month:02} monday_first={monday_first}"
                    );
                    for pair in grid.windows(2) {
                        assert_eq!(
                            pair[1].date,
                            pair[0].date.checked_add_days(Days::new(1)).unwrap()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn monday_first_vs_sunday_first_alignment() {
        // 2026-11-01 is a Sunday; 2026-11-23 is a Monday.
        let today = d(2026, 11, 23);

        let mon_grid = build_month_grid(2026, 11, today, today, true, &[]);
        assert_eq!(mon_grid[0].date.weekday(), Weekday::Mon);
        assert_eq!(mon_grid[0].date, d(2026, 10, 26));
        assert!(!mon_grid[0].in_current_month);
        assert_eq!(mon_grid[6].date, d(2026, 11, 1));
        assert!(mon_grid[6].in_current_month);
        assert!(mon_grid[28].is_today);
        assert!(mon_grid[28].is_selected);

        let sun_grid = build_month_grid(2026, 11, today, today, false, &[]);
        assert_eq!(sun_grid[0].date.weekday(), Weekday::Sun);
        assert_eq!(sun_grid[0].date, d(2026, 11, 1));
        assert!(sun_grid[0].in_current_month);
        assert_eq!(sun_grid[22].date, d(2026, 11, 23));
        assert!(sun_grid[22].is_today);
    }

    #[test]
    fn store_toml_roundtrip_event_crud_and_malformed_fallback() {
        let mut store = CalendarStore {
            monday_first: true,
            ..CalendarStore::default()
        };

        assert_eq!(store.add_event(d(2026, 11, 23), "   "), None);
        let id1 = store
            .add_event(d(2026, 11, 23), "  Design sync  ")
            .expect("first id");
        let id2 = store
            .add_event(d(2026, 11, 23), "Ship calendar")
            .expect("second id");
        assert_ne!(id1, id2);

        store.toggle_event(id1);
        assert!(store.events_for_date(d(2026, 11, 23))[0].done);

        let grid = build_month_grid(
            2026,
            11,
            d(2026, 11, 23),
            d(2026, 11, 23),
            store.monday_first,
            &store.events,
        );
        let cell = grid.iter().find(|c| c.date == d(2026, 11, 23)).unwrap();
        assert_eq!(cell.event_count, 2);

        let dir = std::env::temp_dir().join(format!("vita_cal_test_{}", std::process::id()));
        let file = dir.join("calendar.toml");
        store.save_atomic_to(&file).expect("save calendar store");

        let raw = fs::read_to_string(&file).expect("read saved file");
        let parsed = CalendarStore::from_toml_str(&raw).expect("parse valid toml");
        assert_eq!(store, parsed);

        store.delete_event(id1);
        assert_eq!(store.events_for_date(d(2026, 11, 23)).len(), 1);

        assert!(CalendarStore::from_toml_str("not = [valid = toml").is_err());
        fs::write(&file, "not = [valid = toml").expect("write malformed file");
        let fallback = CalendarStore::load_or_default_from(&file);
        assert_eq!(fallback, CalendarStore::default());

        let _ = fs::remove_dir_all(dir);
    }
}
