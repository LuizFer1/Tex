use crate::eval::Outcome;
use crate::lexer::{Tok, Token};
use crate::units::Unit;
use crate::{Answer, AnswerKind, Locale};

/// The query as the card shows it: the typed spacing, `×`, `÷` and `→` for the operators.
pub fn expression(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        if token.space_before && !out.is_empty() {
            out.push(' ');
        }
        match token.tok {
            Tok::Op('*') => out.push('×'),
            Tok::Op('/') => out.push('÷'),
            Tok::Arrow => out.push('→'),
            _ => out.push_str(&token.text),
        }
    }
    out
}

/// Up to 10 significant digits, trailing zeros dropped, scientific notation outside
/// 1e-9..1e15, the locale's separators; `group` adds the thousands separator.
pub fn number(x: f64, locale: Locale, group: bool) -> String {
    let (decimal, separator) = match locale {
        Locale::Pt => (',', '.'),
        Locale::En => ('.', ','),
    };
    let x = if x == 0.0 { 0.0 } else { x };
    let abs = x.abs();
    if abs != 0.0 && !(1e-9..1e15).contains(&abs) {
        let text = format!("{x:.9e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        return format!(
            "{}e{exponent}",
            trim(mantissa).replace('.', &decimal.to_string())
        );
    }
    let decimals = if abs == 0.0 {
        0
    } else {
        (9 - abs.log10().floor() as i32).clamp(0, 18) as usize
    };
    let text = format!("{x:.decimals$}");
    let text = trim(&text);
    let (int, frac) = match text.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (text, None),
    };
    let (sign, digits) = match int.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", int),
    };
    let digits = if group {
        group_digits(digits, separator)
    } else {
        digits.to_owned()
    };
    let mut out = format!("{sign}{digits}");
    if let Some(frac) = frac {
        out.push(decimal);
        out.push_str(frac);
    }
    if out == "-0" { "0".to_owned() } else { out }
}

fn trim(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

fn group_digits(digits: &str, separator: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

pub fn quantity(x: f64, unit: &Unit, locale: Locale, group: bool) -> String {
    let value = number(x, locale, group);
    if unit.symbol == "°" {
        format!("{value}°")
    } else {
        let singular = number(x.abs(), locale, false) == "1";
        format!("{value} {}", unit.symbol_for(locale, singular))
    }
}

/// Seconds as days, hours, minutes and seconds (`2 h 25 min`); under a minute, plain seconds.
pub fn duration(seconds: f64, locale: Locale) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let total = seconds.abs();
    if total < 60.0 {
        return format!("{sign}{} s", number(total, locale, false));
    }
    let mut rest = total.round() as u64;
    let mut parts = Vec::new();
    for (size, symbol) in [(86400, "d"), (3600, "h"), (60, "min"), (1, "s")] {
        let count = rest / size;
        rest %= size;
        if count > 0 {
            parts.push(format!("{count} {symbol}"));
        }
    }
    format!("{sign}{}", parts.join(" "))
}

pub fn answer(tokens: &[Token], outcome: Outcome, locale: Locale) -> Answer {
    let (result, copy, kind) = match outcome {
        Outcome::Number(x) => (
            number(x, locale, true),
            number(x, locale, false),
            AnswerKind::Number,
        ),
        Outcome::Percent(x) => (
            format!("{}%", number(x, locale, true)),
            format!("{}%", number(x, locale, false)),
            AnswerKind::Number,
        ),
        Outcome::Quantity(x, unit) => (
            quantity(x, unit, locale, true),
            quantity(x, unit, locale, false),
            AnswerKind::Quantity,
        ),
        Outcome::Duration(seconds) => {
            let text = duration(seconds, locale);
            (text.clone(), text, AnswerKind::Duration)
        }
    };
    Answer {
        expression: expression(tokens),
        result,
        copy,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(number(1234567.891, Locale::Pt, true), "1.234.567,891");
        assert_eq!(number(1234567.891, Locale::En, true), "1,234,567.891");
        assert_eq!(number(1234567.891, Locale::Pt, false), "1234567,891");
        assert_eq!(number(-0.0, Locale::Pt, true), "0");
        assert_eq!(number(-1500.0, Locale::Pt, true), "-1.500");
        assert_eq!(number(0.1 + 0.2, Locale::En, true), "0.3");
        assert_eq!(number(1.5e15, Locale::Pt, true), "1,5e15");
        assert_eq!(number(1.5e-10, Locale::En, true), "1.5e-10");
        assert_eq!(number(100.0, Locale::Pt, true), "100");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(8700.0, Locale::Pt), "2 h 25 min");
        assert_eq!(duration(0.0, Locale::Pt), "0 s");
        assert_eq!(duration(1.5, Locale::Pt), "1,5 s");
        assert_eq!(duration(-90.0, Locale::En), "-1 min 30 s");
        assert_eq!(duration(86400.0, Locale::En), "1 d");
    }
}
