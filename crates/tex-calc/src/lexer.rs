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
        let (tok, text) = if c.is_ascii_digit()
            || (matches!(c, '.' | ',') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            if is_iso_date(&chars[i..]) {
                return None;
            }
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
                    if !chars.get(i + 1).is_some_and(char::is_ascii_digit) =>
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
    Some(out)
}

fn is_iso_date(chars: &[char]) -> bool {
    let pattern = "dddd-dd-dd";
    chars.len() >= pattern.len()
        && pattern.chars().zip(chars).all(|(p, c)| match p {
            'd' => c.is_ascii_digit(),
            _ => *c == '-',
        })
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
        let grouped =
            (1..=3).contains(&groups[0].len()) && groups[1..].iter().all(|group| group.len() == 3);
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
        assert_eq!(tokens("2024-10-19", Locale::Pt), None);
        assert_eq!(tokens("1.2.3", Locale::Pt), None);
    }
}
