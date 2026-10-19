use crate::Locale;
use crate::units;

pub const FUNCTION_NAMES: &[&str] = &[
    "sqrt", "raiz", "cbrt", "sin", "sen", "cos", "tan", "tg", "asin", "acos", "atan", "ln", "log",
    "log2", "abs", "round", "arred", "floor", "piso", "ceil", "teto", "exp",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Word,
    Op(char),
    Arrow,
    Day {
        year: Option<i16>,
        month: i8,
        day: i8,
    },
    Clock {
        hour: i8,
        minute: i8,
    },
}

/// One token. `text` is what was typed (operators already mapped: `×` is `*`, `**` is `^`),
/// `folded` is `text` without case or accents, `space_before` keeps the typed spacing.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub text: String,
    pub folded: String,
    pub space_before: bool,
}

/// Splits a query into tokens, or `None` when it has a character or number the calculator
/// does not read (so file names and paths never become calculations).
pub fn tokens(query: &str, locale: Locale) -> Option<Vec<Token>> {
    let chars: Vec<char> = query.chars().collect();
    let mut out = Vec::new();
    let mut space = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            space = true;
            i += 1;
            continue;
        }
        let start = i;
        let special = if c.is_ascii_digit() {
            special(&chars[i..], locale)
        } else {
            None
        };
        let (tok, text) = if let Some(found) = special {
            let (tok, length) = found?;
            i += length;
            (tok, chars[start..i].iter().collect())
        } else if c.is_ascii_digit()
            || (matches!(c, '.' | ',') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            while i < chars.len() && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | ',')) {
                i += 1;
            }
            let mut value = number(&chars[start..i].iter().collect::<String>(), locale)?;
            if matches!(chars.get(i), Some('e' | 'E')) {
                let mut j = i + 1;
                if matches!(chars.get(j), Some('+' | '-')) {
                    j += 1;
                }
                let digits = j;
                while chars.get(j).is_some_and(char::is_ascii_digit) {
                    j += 1;
                }
                if j > digits {
                    let exponent: i32 = chars[i + 1..j].iter().collect::<String>().parse().ok()?;
                    value *= 10f64.powi(exponent);
                    i = j;
                }
            }
            (Tok::Num(value), chars[start..i].iter().collect())
        } else if c.is_alphabetic() || matches!(c, '°' | 'º') {
            i += 1;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let mut word: String = chars[start..i]
                .iter()
                .map(|&c| if c == 'º' { '°' } else { c })
                .collect();
            match chars.get(i) {
                Some('²') => {
                    word.push('2');
                    i += 1;
                }
                Some('³') => {
                    word.push('3');
                    i += 1;
                }
                Some(&digit @ ('2' | '3'))
                    if !chars.get(i + 1).is_some_and(|c| c.is_alphanumeric()) =>
                {
                    let candidate = format!("{word}{digit}");
                    if units::lookup(&candidate).is_some()
                        || FUNCTION_NAMES.contains(&tex_index::fold(&candidate).as_str())
                    {
                        word = candidate;
                        i += 1;
                    }
                }
                _ => {}
            }
            if chars.get(i) == Some(&'/') {
                let mut j = i + 1;
                while chars.get(j).is_some_and(|c| c.is_alphabetic()) {
                    j += 1;
                }
                if j > i + 1 {
                    let candidate =
                        format!("{word}/{}", chars[i + 1..j].iter().collect::<String>());
                    if units::lookup(&candidate).is_some() {
                        word = candidate;
                        i = j;
                    }
                }
            }
            (Tok::Word, word)
        } else {
            i += 1;
            let tok = match c {
                '+' | '(' | ')' | '%' | '!' | '^' => Tok::Op(c),
                '-' | '−' if chars.get(i) == Some(&'>') => {
                    i += 1;
                    Tok::Arrow
                }
                '-' | '−' => Tok::Op('-'),
                '→' => Tok::Arrow,
                '*' if chars.get(i) == Some(&'*') => {
                    i += 1;
                    Tok::Op('^')
                }
                '*' | '×' | '·' => Tok::Op('*'),
                '/' | '÷' => Tok::Op('/'),
                _ => return None,
            };
            let text = match tok {
                Tok::Op(op) => op.to_string(),
                _ => "->".to_owned(),
            };
            (tok, text)
        };
        let folded = if tok == Tok::Word {
            tex_index::fold(&text)
        } else {
            text.clone()
        };
        out.push(Token {
            tok,
            text,
            folded,
            space_before: space,
        });
        space = false;
    }
    (!looks_like_text(&out)).then_some(out)
}

fn bare_integer(token: &Token) -> Option<&str> {
    (matches!(token.tok, Tok::Num(_)) && token.text.bytes().all(|b| b.is_ascii_digit()))
        .then_some(token.text.as_str())
}

/// Whether the integers are written like codes, dates or phone numbers rather than a
/// calculation: a leading zero (`03`), an unspaced `-` next to an integer of four or more
/// digits (`2024-10`, `555-1234`), or two unspaced `-` or `/` in a row (`19/10/2026`).
fn looks_like_text(tokens: &[Token]) -> bool {
    if tokens
        .iter()
        .filter_map(bare_integer)
        .any(|text| text.len() > 1 && text.starts_with('0'))
    {
        return true;
    }
    let joined = |i: usize, ops: &[char]| {
        tokens
            .get(i + 1)
            .is_some_and(|op| matches!(op.tok, Tok::Op(c) if ops.contains(&c)) && !op.space_before)
            && tokens
                .get(i + 2)
                .is_some_and(|right| !right.space_before && bare_integer(right).is_some())
            && bare_integer(&tokens[i]).is_some()
    };
    (0..tokens.len()).any(|i| {
        let long = |j: usize| bare_integer(&tokens[j]).is_some_and(|text| text.len() >= 4);
        (joined(i, &['-']) && (long(i) || long(i + 2)))
            || (joined(i, &['-', '/']) && joined(i + 2, &['-', '/']))
    })
}

/// Dates (`2026-10-19`; `31/03` and `31/03/2027` day first in Portuguese, month first in
/// English, two digits each without a year so `1/2` stays a fraction) and times of day
/// (`9:00`, `17:30`, `9am`, `5:30 pm`) at the start of `chars`: the token and its length,
/// `Some(None)` when the shape matches but the value is invalid.
fn special(chars: &[char], locale: Locale) -> Option<Option<(Tok, usize)>> {
    let run = |from: usize| {
        chars.get(from..).map_or(0, |rest| {
            rest.iter().take_while(|c| c.is_ascii_digit()).count()
        })
    };
    let value = |from: usize, length: usize| -> i32 {
        chars[from..from + length]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    let digit_at = |at: usize| chars.get(at).is_some_and(char::is_ascii_digit);
    let first = run(0);
    if first == 4
        && chars.get(4) == Some(&'-')
        && run(5) == 2
        && chars.get(7) == Some(&'-')
        && run(8) == 2
        && !digit_at(10)
    {
        let (year, month, day) = (value(0, 4), value(5, 2), value(8, 2));
        return Some(day_token(Some(year), month, day).map(|tok| (tok, 10)));
    }
    if (1..=2).contains(&first) && chars.get(first) == Some(&'/') {
        let second = run(first + 1);
        if (1..=2).contains(&second) {
            let end = first + 1 + second;
            let year = (chars.get(end) == Some(&'/')
                && run(end + 1) == 4
                && !digit_at(end + 5)
                && chars.get(end + 5) != Some(&'/'))
            .then(|| value(end + 1, 4));
            let length = if year.is_some() { end + 5 } else { end };
            let plain = year.is_none()
                && first == 2
                && second == 2
                && !digit_at(end)
                && chars.get(end) != Some(&'/');
            if year.is_some() || plain {
                let (a, b) = (value(0, first), value(first + 1, second));
                let (day, month) = match locale {
                    Locale::Pt => (a, b),
                    Locale::En => (b, a),
                };
                return Some(day_token(year, month, day).map(|tok| (tok, length)));
            }
        }
    }
    if !(1..=2).contains(&first) {
        return None;
    }
    let (mut hour, mut minute, mut length) = (value(0, first), 0, first);
    let colon = chars.get(first) == Some(&':') && run(first + 1) == 2 && !digit_at(first + 3);
    if colon {
        minute = value(first + 1, 2);
        length = first + 3;
    }
    let gap = usize::from(chars.get(length) == Some(&' '));
    let half: String = chars
        .get(length + gap..length + gap + 2)
        .map(|pair| pair.iter().collect::<String>().to_ascii_lowercase())
        .unwrap_or_default();
    let after = chars
        .get(length + gap + 2)
        .is_some_and(|c| c.is_alphanumeric());
    if (half == "am" || half == "pm") && !after {
        if !(1..=12).contains(&hour) || minute > 59 {
            return Some(None);
        }
        hour = hour % 12 + if half == "pm" { 12 } else { 0 };
        length += gap + 2;
    } else if !colon {
        return None;
    } else if hour > 23 || minute > 59 {
        return Some(None);
    }
    Some(Some((
        Tok::Clock {
            hour: i8::try_from(hour).ok()?,
            minute: i8::try_from(minute).ok()?,
        },
        length,
    )))
}

fn day_token(year: Option<i32>, month: i32, day: i32) -> Option<Tok> {
    let month = i8::try_from(month).ok()?;
    let day = i8::try_from(day).ok()?;
    let year = year.map(i16::try_from).transpose().ok()?;
    jiff::civil::Date::new(year.unwrap_or(2024), month, day).ok()?;
    Some(Tok::Day { year, month, day })
}

/// Reads a run of digits, `.` and `,` with the decimal separator of `locale`. The other
/// separator groups thousands only in full groups of three (`1.500` is 1500 in Portuguese);
/// used once without full groups it is a decimal point too (`1.5` is 1.5 in Portuguese).
pub fn number(text: &str, locale: Locale) -> Option<f64> {
    let (decimal, group) = match locale {
        Locale::Pt => (',', '.'),
        Locale::En => ('.', ','),
    };
    let mut parts = text.split(decimal);
    let int = parts.next()?;
    let frac = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if let Some(frac) = frac
        && (frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let int = if int.contains(group) {
        let groups: Vec<&str> = int.split(group).collect();
        let grouped = (1..=3).contains(&groups[0].len())
            && !groups[0].starts_with('0')
            && groups[1..].iter().all(|group| group.len() == 3);
        if grouped {
            groups.concat()
        } else if groups.len() == 2 && frac.is_none() && !groups[1].is_empty() {
            return format!("{}.{}", groups[0], groups[1]).parse().ok();
        } else {
            return None;
        }
    } else {
        int.to_owned()
    };
    match frac {
        Some(frac) => format!("{int}.{frac}").parse().ok(),
        None => int.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(query: &str) -> Vec<Tok> {
        tokens(query, Locale::Pt)
            .unwrap()
            .into_iter()
            .map(|token| token.tok)
            .collect()
    }

    fn texts(query: &str) -> Vec<String> {
        tokens(query, Locale::Pt)
            .unwrap()
            .into_iter()
            .map(|token| token.text)
            .collect()
    }

    #[test]
    fn numbers_follow_the_locale() {
        assert_eq!(number("1.500", Locale::Pt), Some(1500.0));
        assert_eq!(number("1.5", Locale::Pt), Some(1.5));
        assert_eq!(number("12.50", Locale::Pt), Some(12.5));
        assert_eq!(number("0.500", Locale::Pt), Some(0.5));
        assert_eq!(number("0,500", Locale::En), Some(0.5));
        assert_eq!(number("1.234.567,8", Locale::Pt), Some(1234567.8));
        assert_eq!(number("1,5", Locale::Pt), Some(1.5));
        assert_eq!(number(",5", Locale::Pt), Some(0.5));
        assert_eq!(number("1,000", Locale::En), Some(1000.0));
        assert_eq!(number("1,5", Locale::En), Some(1.5));
        assert_eq!(number("1,234.5", Locale::En), Some(1234.5));
        assert_eq!(number("1.2.3", Locale::Pt), None);
        assert_eq!(number("5.", Locale::Pt), None);
        assert_eq!(number("1,2,3", Locale::Pt), None);
        assert_eq!(number("1,2,3", Locale::En), None);
    }

    #[test]
    fn operators_are_mapped() {
        assert_eq!(kinds("2**3"), [Tok::Num(2.0), Tok::Op('^'), Tok::Num(3.0)]);
        assert_eq!(
            kinds("6 × 2 ÷ 3"),
            [
                Tok::Num(6.0),
                Tok::Op('*'),
                Tok::Num(2.0),
                Tok::Op('/'),
                Tok::Num(3.0)
            ]
        );
        assert_eq!(kinds("5 −> 3"), [Tok::Num(5.0), Tok::Arrow, Tok::Num(3.0)]);
        assert_eq!(kinds("1e3"), [Tok::Num(1000.0)]);
        assert_eq!(kinds("2e"), [Tok::Num(2.0), Tok::Word]);
    }

    #[test]
    fn units_stay_in_one_word() {
        assert_eq!(texts("10ft"), ["10", "ft"]);
        assert_eq!(texts("5 m²"), ["5", "m2"]);
        assert_eq!(texts("5 m2"), ["5", "m2"]);
        assert_eq!(texts("100 km/h"), ["100", "km/h"]);
        assert_eq!(texts("100 ºC"), ["100", "°C"]);
        assert_eq!(texts("30°"), ["30", "°"]);
        assert_eq!(texts("log2 8"), ["log2", "8"]);
        assert_eq!(texts("5ft2in"), ["5", "ft", "2", "in"]);
        assert_eq!(texts("2h30min"), ["2", "h", "30", "min"]);
        assert_eq!(texts("a/b"), ["a", "/", "b"]);
    }

    #[test]
    fn spacing_is_kept() {
        let tokens = tokens("2 +3", Locale::Pt).unwrap();
        assert!(tokens[1].space_before);
        assert!(!tokens[2].space_before);
    }

    #[test]
    fn unknown_text_is_refused() {
        assert_eq!(tokens("c:\\users", Locale::Pt), None);
        assert_eq!(tokens("notes.txt", Locale::Pt), None);
        assert_eq!(tokens("1.2.3", Locale::Pt), None);
    }

    #[test]
    fn dates_and_times_of_day() {
        let pt = |query: &str| {
            tokens(query, Locale::Pt).map(|tokens| {
                tokens
                    .into_iter()
                    .map(|token| token.tok)
                    .collect::<Vec<_>>()
            })
        };
        let en = |query: &str| {
            tokens(query, Locale::En).map(|tokens| {
                tokens
                    .into_iter()
                    .map(|token| token.tok)
                    .collect::<Vec<_>>()
            })
        };
        let day = |year, month, day| Tok::Day { year, month, day };
        let clock = |hour, minute| Tok::Clock { hour, minute };
        assert_eq!(pt("2026-10-19"), Some(vec![day(Some(2026), 10, 19)]));
        assert_eq!(pt("31/03"), Some(vec![day(None, 3, 31)]));
        assert_eq!(en("03/31"), Some(vec![day(None, 3, 31)]));
        assert_eq!(pt("31/03/2027"), Some(vec![day(Some(2027), 3, 31)]));
        assert_eq!(pt("19/10/2026"), Some(vec![day(Some(2026), 10, 19)]));
        assert_eq!(pt("1/2/2027"), Some(vec![day(Some(2027), 2, 1)]));
        assert_eq!(
            pt("1/2"),
            Some(vec![Tok::Num(1.0), Tok::Op('/'), Tok::Num(2.0)])
        );
        assert_eq!(pt("9:00"), Some(vec![clock(9, 0)]));
        assert_eq!(pt("17:30"), Some(vec![clock(17, 30)]));
        assert_eq!(pt("9am"), Some(vec![clock(9, 0)]));
        assert_eq!(pt("5:30pm"), Some(vec![clock(17, 30)]));
        assert_eq!(pt("12am"), Some(vec![clock(0, 0)]));
        assert_eq!(pt("12 pm"), Some(vec![clock(12, 0)]));
        assert_eq!(pt("9 amanha").map(|tokens| tokens.len()), Some(2));
        for query in [
            "2024-13-45",
            "31/02",
            "45/99",
            "25:00",
            "13pm",
            "9:5",
            "9:",
            "2024-1",
        ] {
            assert_eq!(pt(query), None, "{query}");
        }
    }

    #[test]
    fn codes_and_dates_are_refused() {
        for query in ["03 + 1", "2024-10", "555-1234", "1/2/3", "123-45-6789"] {
            assert_eq!(tokens(query, Locale::Pt), None, "{query}");
        }
        for query in ["0", "0,5", "10-5", "1000/4", "1500 - 200", "2024 -10"] {
            assert!(tokens(query, Locale::Pt).is_some(), "{query}");
        }
    }
}
