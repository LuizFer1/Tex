use jiff::ToSpan;
use jiff::civil::{Date, Time};

use crate::Locale;

const MONTHS: &[(&str, i8)] = &[
    ("janeiro", 1),
    ("jan", 1),
    ("january", 1),
    ("fevereiro", 2),
    ("fev", 2),
    ("february", 2),
    ("feb", 2),
    ("marco", 3),
    ("mar", 3),
    ("march", 3),
    ("abril", 4),
    ("abr", 4),
    ("april", 4),
    ("apr", 4),
    ("maio", 5),
    ("mai", 5),
    ("may", 5),
    ("junho", 6),
    ("jun", 6),
    ("june", 6),
    ("julho", 7),
    ("jul", 7),
    ("july", 7),
    ("agosto", 8),
    ("ago", 8),
    ("august", 8),
    ("aug", 8),
    ("setembro", 9),
    ("set", 9),
    ("september", 9),
    ("sep", 9),
    ("sept", 9),
    ("outubro", 10),
    ("out", 10),
    ("october", 10),
    ("oct", 10),
    ("novembro", 11),
    ("nov", 11),
    ("november", 11),
    ("dezembro", 12),
    ("dez", 12),
    ("december", 12),
    ("dec", 12),
];

pub fn month(folded: &str) -> Option<i8> {
    MONTHS
        .iter()
        .find(|(name, _)| *name == folded)
        .map(|(_, month)| *month)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YearRule {
    Next,
    Last,
}

/// The `month`/`day` date nearest to `today` going forward (`Next`) or back (`Last`), today
/// included; looks up to 8 years away so that 29 February finds a leap year.
pub fn resolve(month: i8, day: i8, today: Date, rule: YearRule) -> Option<Date> {
    let step: i16 = match rule {
        YearRule::Next => 1,
        YearRule::Last => -1,
    };
    (0..8).find_map(|k| {
        Date::new(today.year() + step * k, month, day)
            .ok()
            .filter(|date| match rule {
                YearRule::Next => *date >= today,
                YearRule::Last => *date <= today,
            })
    })
}

/// Monday-to-Friday days `d` with `from < d <= to`; negative when `to` comes before `from`.
pub fn workdays(from: Date, to: Date) -> i64 {
    if to < from {
        return -workdays(to, from);
    }
    let days = i64::from((to - from).get_days());
    let start = i64::from(from.weekday().to_monday_one_offset());
    let partial = (1..=days % 7)
        .filter(|k| (start - 1 + k) % 7 + 1 <= 5)
        .count() as i64;
    days / 7 * 5 + partial
}

/// `date` moved by a whole number of days, weeks, months or years (`symbol` is the unit's
/// symbol); `None` for fractions, smaller units or a date out of range.
pub fn shift(date: Date, value: f64, symbol: &str) -> Option<Date> {
    if value.fract() != 0.0 || value.abs() > 1e6 {
        return None;
    }
    let n = value as i64;
    let span = match symbol {
        "d" => n.days(),
        "sem" => n.weeks(),
        "mês" => n.months(),
        "ano" => n.years(),
        _ => return None,
    };
    date.checked_add(span).ok()
}

/// A time of day moved by `seconds`, wrapping around midnight.
pub fn shift_time(time: Time, seconds: f64) -> Option<Time> {
    if !seconds.is_finite() || seconds.abs() > 1e12 {
        return None;
    }
    let start =
        i64::from(time.hour()) * 3600 + i64::from(time.minute()) * 60 + i64::from(time.second());
    let total = (start + seconds.round() as i64).rem_euclid(86400);
    Time::new(
        (total / 3600) as i8,
        (total % 3600 / 60) as i8,
        (total % 60) as i8,
        0,
    )
    .ok()
}

pub fn seconds_of(time: Time) -> f64 {
    f64::from(time.hour()) * 3600.0 + f64::from(time.minute()) * 60.0 + f64::from(time.second())
}

pub fn weekday(date: Date) -> u8 {
    date.weekday().to_monday_one_offset() as u8
}

pub fn format_date(date: Date, locale: Locale) -> String {
    match locale {
        Locale::Pt => format!("{:02}/{:02}/{}", date.day(), date.month(), date.year()),
        Locale::En => format!("{:02}/{:02}/{}", date.month(), date.day(), date.year()),
    }
}

pub fn format_time(time: Time, locale: Locale) -> String {
    match locale {
        Locale::Pt => format!("{:02}:{:02}", time.hour(), time.minute()),
        Locale::En => {
            let hour = match time.hour() % 12 {
                0 => 12,
                hour => hour,
            };
            let half = if time.hour() < 12 { "AM" } else { "PM" };
            format!("{hour}:{:02} {half}", time.minute())
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::{date, time};

    use super::*;

    const TODAY: Date = date(2026, 10, 19);

    #[test]
    fn dates_without_a_year() {
        assert_eq!(
            resolve(3, 31, TODAY, YearRule::Next),
            Some(date(2027, 3, 31))
        );
        assert_eq!(
            resolve(3, 31, TODAY, YearRule::Last),
            Some(date(2026, 3, 31))
        );
        assert_eq!(resolve(10, 19, TODAY, YearRule::Next), Some(TODAY));
        assert_eq!(resolve(10, 19, TODAY, YearRule::Last), Some(TODAY));
        assert_eq!(
            resolve(2, 29, TODAY, YearRule::Next),
            Some(date(2028, 2, 29))
        );
        assert_eq!(resolve(2, 30, TODAY, YearRule::Next), None);
    }

    #[test]
    fn workdays_count_monday_to_friday() {
        assert_eq!(workdays(TODAY, date(2027, 3, 31)), 117);
        assert_eq!(workdays(date(2027, 3, 31), TODAY), -117);
        assert_eq!(workdays(TODAY, date(2026, 10, 24)), 4);
        assert_eq!(workdays(date(2026, 10, 23), date(2026, 10, 26)), 1);
        assert_eq!(workdays(TODAY, TODAY), 0);
    }

    #[test]
    fn shifts() {
        assert_eq!(shift(TODAY, 30.0, "d"), Some(date(2026, 11, 18)));
        assert_eq!(shift(TODAY, -2.0, "sem"), Some(date(2026, 10, 5)));
        assert_eq!(
            shift(date(2026, 10, 31), 1.0, "mês"),
            Some(date(2026, 11, 30))
        );
        assert_eq!(shift(TODAY, 1.0, "ano"), Some(date(2027, 10, 19)));
        assert_eq!(shift(TODAY, 1.5, "d"), None);
        assert_eq!(shift(TODAY, 2.0, "h"), None);
        assert_eq!(
            shift_time(time(23, 0, 0, 0), 7200.0),
            Some(time(1, 0, 0, 0))
        );
        assert_eq!(
            shift_time(time(0, 30, 0, 0), -3600.0),
            Some(time(23, 30, 0, 0))
        );
    }

    #[test]
    fn formats() {
        assert_eq!(format_date(TODAY, Locale::Pt), "19/10/2026");
        assert_eq!(format_date(TODAY, Locale::En), "10/19/2026");
        assert_eq!(format_time(time(9, 5, 0, 0), Locale::Pt), "09:05");
        assert_eq!(format_time(time(0, 0, 0, 0), Locale::En), "12:00 AM");
        assert_eq!(format_time(time(12, 30, 0, 0), Locale::En), "12:30 PM");
        assert_eq!(format_time(time(17, 30, 0, 0), Locale::En), "5:30 PM");
        assert_eq!(weekday(TODAY), 1);
        assert_eq!(month("marco"), Some(3));
        assert_eq!(month("sept"), Some(9));
        assert_eq!(month("xyz"), None);
    }
}
