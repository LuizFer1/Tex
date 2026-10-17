use std::collections::HashSet;

use tex_platform::LocalTime;

/// A size as a number and a unit (0 B, 1 KB, 2 MB, 3 GB), for Slint to build the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeParts {
    pub value: String,
    pub unit: i32,
}

/// One decimal below 10 (except bytes), a whole number above, with the language's separator.
pub fn size_parts(bytes: u64, decimal: char) -> SizeParts {
    if bytes < 1024 {
        return SizeParts {
            value: bytes.to_string(),
            unit: 0,
        };
    }
    let mut unit = 1;
    let mut value = bytes as f64 / 1024.0;
    while value.round() >= 1024.0 && unit < 3 {
        value /= 1024.0;
        unit += 1;
    }
    let tenths = (value * 10.0).round() as u64;
    let value = if tenths < 100 {
        format!("{}{decimal}{}", tenths / 10, tenths % 10)
    } else {
        (value.round() as u64).to_string()
    };
    SizeParts { value, unit }
}

/// When an entry was copied: `day` is 0 today, 1 yesterday, -1 another day; `date` is
/// `[day, month, year]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenParts {
    pub day: i32,
    pub time: String,
    pub date: [i32; 3],
}

pub fn when_parts(copied: LocalTime, now: LocalTime) -> WhenParts {
    let day = match days(now) - days(copied) {
        0 => 0,
        1 => 1,
        _ => -1,
    };
    WhenParts {
        day,
        time: format!("{:02}:{:02}", copied.hour, copied.minute),
        date: [
            i32::from(copied.day),
            i32::from(copied.month),
            i32::from(copied.year),
        ],
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar.
fn days(time: LocalTime) -> i64 {
    let month = i64::from(time.month);
    let year = i64::from(time.year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(time.day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Ids of the rows on screen, newest first, and the selected one.
#[derive(Debug, Default)]
pub struct HistoryList {
    ids: Vec<i64>,
    selected: Option<i64>,
}

impl HistoryList {
    /// Shows a new list and selects its first row.
    pub fn replace(&mut self, ids: Vec<i64>) {
        self.selected = ids.first().copied();
        self.ids = ids;
    }

    pub fn append(&mut self, ids: Vec<i64>) {
        self.ids.extend(ids);
        if self.selected.is_none() {
            self.selected = self.ids.first().copied();
        }
    }

    /// Adds a row on top without moving the selection (an empty list selects it).
    pub fn insert_top(&mut self, id: i64) {
        self.ids.insert(0, id);
        if self.selected.is_none() {
            self.selected = Some(id);
        }
    }

    /// Removes rows. A removed selection moves to the next row, or the previous one at the end.
    /// Returns the selected index afterwards.
    pub fn remove(&mut self, ids: &[i64]) -> Option<usize> {
        let gone: HashSet<i64> = ids.iter().copied().collect();
        if let Some(selected) = self.selected
            && gone.contains(&selected)
        {
            let index = self.index_of(selected).unwrap_or(0);
            let next = self.ids[index..].iter().find(|id| !gone.contains(id));
            let previous = self.ids[..index].iter().rev().find(|id| !gone.contains(id));
            self.selected = next.or(previous).copied();
        }
        self.ids.retain(|id| !gone.contains(id));
        self.selected_index()
    }

    pub fn select_index(&mut self, index: usize) {
        if let Some(&id) = self.ids.get(index) {
            self.selected = Some(id);
        }
    }

    /// Moves the selection by `step` rows, stopping at the ends.
    pub fn step(&mut self, step: isize) {
        let Some(last) = self.ids.len().checked_sub(1) else {
            return;
        };
        let index = match self.selected_index() {
            Some(index) => index.saturating_add_signed(step).min(last),
            None if step < 0 => last,
            None => 0,
        };
        self.selected = Some(self.ids[index]);
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.index_of(self.selected?)
    }

    pub fn selected(&self) -> Option<i64> {
        self.selected
    }

    pub fn index_of(&self, id: i64) -> Option<usize> {
        self.ids.iter().position(|&other| other == id)
    }

    pub fn last(&self) -> Option<i64> {
        self.ids.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    fn size(value: &str, unit: i32) -> SizeParts {
        SizeParts {
            value: value.into(),
            unit,
        }
    }

    fn list(ids: &[i64]) -> HistoryList {
        let mut list = HistoryList::default();
        list.replace(ids.to_vec());
        list
    }

    #[test]
    fn sizes_in_bytes_have_no_decimals() {
        assert_eq!(size_parts(0, ','), size("0", 0));
        assert_eq!(size_parts(1023, ','), size("1023", 0));
    }

    #[test]
    fn sizes_below_ten_have_one_decimal() {
        assert_eq!(size_parts(1024, ','), size("1,0", 1));
        assert_eq!(size_parts(1536, ','), size("1,5", 1));
        assert_eq!(size_parts(1536, '.'), size("1.5", 1));
        assert_eq!(size_parts(9 * 1024 + 900, '.'), size("9.9", 1));
    }

    #[test]
    fn sizes_from_ten_up_are_whole() {
        assert_eq!(size_parts(10 * 1024 * 1024, ','), size("10", 2));
        assert_eq!(size_parts(9 * 1024 + 1000, ','), size("10", 1));
        assert_eq!(size_parts(312 * 1024 * 1024 + 300_000, ','), size("312", 2));
        assert_eq!(size_parts(5 * 1024 * 1024 * 1024, ','), size("5,0", 3));
        assert_eq!(size_parts(2048 * 1024 * 1024 * 1024, ','), size("2048", 3));
    }

    #[test]
    fn sizes_that_round_up_to_1024_move_to_the_next_unit() {
        assert_eq!(size_parts(1024 * 1024 - 1, ','), size("1,0", 2));
    }

    #[test]
    fn today_shows_the_time() {
        let when = when_parts(at(2026, 10, 16, 9, 5), at(2026, 10, 16, 14, 2));
        assert_eq!(
            when,
            WhenParts {
                day: 0,
                time: "09:05".into(),
                date: [16, 10, 2026],
            }
        );
    }

    #[test]
    fn yesterday_across_days_months_and_years() {
        assert_eq!(
            when_parts(at(2026, 10, 15, 23, 59), at(2026, 10, 16, 0, 1)).day,
            1
        );
        assert_eq!(
            when_parts(at(2026, 9, 30, 8, 0), at(2026, 10, 1, 8, 0)).day,
            1
        );
        assert_eq!(
            when_parts(at(2028, 2, 29, 8, 0), at(2028, 3, 1, 8, 0)).day,
            1
        );
        let new_year = when_parts(at(2025, 12, 31, 22, 10), at(2026, 1, 1, 0, 0));
        assert_eq!(new_year.day, 1);
        assert_eq!(new_year.time, "22:10");
        assert_eq!(new_year.date, [31, 12, 2025]);
    }

    #[test]
    fn other_days_show_the_date() {
        assert_eq!(
            when_parts(at(2026, 10, 14, 8, 0), at(2026, 10, 16, 8, 0)).day,
            -1
        );
        assert_eq!(
            when_parts(at(2025, 10, 15, 8, 0), at(2026, 10, 16, 8, 0)).day,
            -1
        );
        assert_eq!(
            when_parts(at(2026, 10, 17, 8, 0), at(2026, 10, 16, 8, 0)).day,
            -1
        );
        assert_eq!(
            when_parts(at(2026, 3, 2, 7, 30), at(2026, 10, 16, 8, 0)).date,
            [2, 3, 2026]
        );
    }

    #[test]
    fn replace_selects_the_first_row() {
        let full = list(&[9, 8, 7]);
        assert_eq!(full.selected(), Some(9));
        assert_eq!(full.selected_index(), Some(0));
        assert_eq!(full.last(), Some(7));
        let empty = list(&[]);
        assert_eq!(empty.selected(), None);
        assert_eq!(empty.selected_index(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn append_keeps_the_selection() {
        let mut list = list(&[9, 8]);
        list.step(1);
        list.append(vec![7, 6]);
        assert_eq!(list.selected(), Some(8));
        assert_eq!(list.index_of(6), Some(3));
        assert_eq!(list.last(), Some(6));
        let mut empty = HistoryList::default();
        empty.append(vec![5, 4]);
        assert_eq!(empty.selected(), Some(5));
    }

    #[test]
    fn insert_top_keeps_the_selected_row() {
        let mut list = list(&[9, 8]);
        list.step(1);
        list.insert_top(10);
        assert_eq!(list.selected(), Some(8));
        assert_eq!(list.selected_index(), Some(2));
        assert_eq!(list.index_of(10), Some(0));
        let mut empty = HistoryList::default();
        empty.insert_top(1);
        assert_eq!(empty.selected(), Some(1));
    }

    #[test]
    fn removing_other_rows_keeps_the_selected_row() {
        let mut list = list(&[9, 8, 7, 6]);
        list.select_index(2);
        assert_eq!(list.remove(&[9, 6, 42]), Some(1));
        assert_eq!(list.selected(), Some(7));
        assert_eq!(list.index_of(8), Some(0));
    }

    #[test]
    fn removing_the_selected_row_moves_to_the_next() {
        let mut list = list(&[9, 8, 7, 6]);
        list.select_index(1);
        assert_eq!(list.remove(&[8, 7]), Some(1));
        assert_eq!(list.selected(), Some(6));
    }

    #[test]
    fn removing_the_selected_last_row_moves_to_the_previous() {
        let mut list = list(&[9, 8, 7]);
        list.select_index(2);
        assert_eq!(list.remove(&[7]), Some(1));
        assert_eq!(list.selected(), Some(8));
        assert_eq!(list.remove(&[9, 8]), None);
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn step_stops_at_the_ends() {
        let mut list = list(&[9, 8, 7]);
        list.step(-1);
        assert_eq!(list.selected_index(), Some(0));
        list.step(1);
        list.step(1);
        list.step(1);
        assert_eq!(list.selected_index(), Some(2));
        list.step(-5);
        assert_eq!(list.selected_index(), Some(0));
        let mut empty = HistoryList::default();
        empty.step(1);
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn select_index_ignores_rows_that_do_not_exist() {
        let mut list = list(&[9, 8]);
        list.select_index(1);
        list.select_index(5);
        assert_eq!(list.selected(), Some(8));
        assert_eq!(list.index_of(42), None);
    }
}
