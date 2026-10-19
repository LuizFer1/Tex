use tex_calc::{AnswerKind, Context, Locale, evaluate};

const NOW: &str = "2026-10-19T10:00:00-03:00[America/Sao_Paulo]";

fn context(locale: Locale) -> Context {
    Context {
        locale,
        now: NOW.parse().unwrap(),
    }
}

fn result(query: &str, locale: Locale) -> Option<String> {
    evaluate(query, &context(locale)).map(|answer| answer.result)
}

fn kind(query: &str, locale: Locale) -> Option<AnswerKind> {
    evaluate(query, &context(locale)).map(|answer| answer.kind)
}

fn check(locale: Locale, cases: &[(&str, &str)]) {
    for (query, expected) in cases {
        assert_eq!(result(query, locale).as_deref(), Some(*expected), "{query}");
    }
}

fn time(place: &str, weekday: u8, offset_minutes: i32) -> Option<AnswerKind> {
    Some(AnswerKind::Time {
        place: place.to_owned(),
        weekday,
        offset_minutes,
    })
}

#[test]
fn dates() {
    check(
        Locale::Pt,
        &[
            ("hoje", "19/10/2026"),
            ("amanhã", "20/10/2026"),
            ("ontem", "18/10/2026"),
            ("dias até 31 mar", "163 d"),
            ("dias até 31 de março de 2027", "163 d"),
            ("dias até 31/03", "163 d"),
            ("dias até 2027-03-31", "163 d"),
            ("dias até 25 dez", "67 d"),
            ("dias desde 1 jan", "291 d"),
            ("dias úteis até 31 mar", "117 d"),
            ("hoje + 30 dias", "18/11/2026"),
            ("hoje + 1 mês", "19/11/2026"),
            ("31 mar - 2 sem", "17/03/2027"),
            ("31 mar - hoje", "163 d"),
        ],
    );
    check(
        Locale::En,
        &[
            ("today", "10/19/2026"),
            ("days until march 31", "163 d"),
            ("days until mar 31 2027", "163 d"),
            ("days until 03/31", "163 d"),
            ("days since jan 1", "291 d"),
            ("workdays until mar 31", "117 d"),
            ("today + 3 months", "01/19/2027"),
        ],
    );
    assert_eq!(
        kind("hoje", Locale::Pt),
        Some(AnswerKind::Date { weekday: 1 })
    );
    assert_eq!(
        kind("hoje + 30 dias", Locale::Pt),
        Some(AnswerKind::Date { weekday: 3 })
    );
    assert_eq!(
        kind("dias até 31 mar", Locale::Pt),
        Some(AnswerKind::Quantity)
    );
}

#[test]
fn times() {
    check(
        Locale::Pt,
        &[
            ("agora", "10:00"),
            ("9:00 + 8h 30min", "17:30"),
            ("17:30 - 9:00", "8 h 30 min"),
            ("23:00 + 2 h", "01:00"),
            ("hora em tóquio", "22:00"),
            ("horas em Tóquio", "22:00"),
            ("hora no japão", "22:00"),
            ("agora em londres", "14:00"),
            ("15:00 em tóquio", "03:00"),
            ("15:00 de tóquio em são paulo", "03:00"),
            ("hora em utc", "13:00"),
            ("hora em utc+3", "16:00"),
            ("hora em gmt-03:00", "10:00"),
        ],
    );
    check(
        Locale::En,
        &[
            ("time in tokyo", "10:00 PM"),
            ("now in new york", "9:00 AM"),
            ("9am + 2h", "11:00 AM"),
            ("5:30pm - 9am", "8 h 30 min"),
            ("3pm tokyo in sao paulo", "3:00 AM"),
            ("15:00 tokyo in sao paulo", "3:00 AM"),
        ],
    );
    assert_eq!(kind("hora em tóquio", Locale::Pt), time("Tóquio", 1, 720));
    assert_eq!(kind("15:00 em tóquio", Locale::Pt), time("Tóquio", 2, 720));
    assert_eq!(
        kind("agora em londres", Locale::Pt),
        time("Londres", 1, 240)
    );
    assert_eq!(kind("hora em utc+3", Locale::Pt), time("UTC+3", 1, 360));
    assert_eq!(
        kind("15:00 tokyo in sao paulo", Locale::En),
        time("São Paulo", 1, 0)
    );
    assert_eq!(kind("9:00 + 1h", Locale::Pt), time("", 1, 0));
    assert_eq!(kind("agora", Locale::Pt), time("", 1, 0));
    assert_eq!(kind("17:30 - 9:00", Locale::Pt), Some(AnswerKind::Duration));
}

#[test]
fn not_date_calculations() {
    let queries = [
        "31 mar",
        "2026-10-19",
        "31/03",
        "19/10/2026",
        "10:30",
        "9am",
        "março",
        "hora",
        "time",
        "tokyo",
        "hora em marte",
        "15:00 em m",
        "25:00 + 1h",
        "31/02",
        "2024-13-45",
        "15:00 tóquio",
        "dias",
        "dias até",
        "hoje * 2",
        "9:00 + 9:00",
        "hoje + 2 h",
        "31 fev - hoje",
        "hoje + 1,5 dias",
        "dias até 9:00",
    ];
    for locale in [Locale::Pt, Locale::En] {
        for query in queries {
            assert_eq!(result(query, locale), None, "{query} ({locale:?})");
        }
    }
}
