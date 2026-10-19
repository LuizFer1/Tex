use tex_calc::{AnswerKind, Context, Locale, evaluate};

fn result(query: &str, locale: Locale) -> Option<String> {
    evaluate(query, &Context { locale }).map(|answer| answer.result)
}

fn check(locale: Locale, cases: &[(&str, &str)]) {
    for (query, expected) in cases {
        assert_eq!(result(query, locale).as_deref(), Some(*expected), "{query}");
    }
}

#[test]
fn math() {
    check(
        Locale::Pt,
        &[
            ("2+2", "4"),
            ("2 * 3", "6"),
            ("7/2", "3,5"),
            ("2^10", "1.024"),
            ("2**3", "8"),
            ("-2^2", "-4"),
            ("2^3^2", "512"),
            ("10 mod 3", "1"),
            ("5!", "120"),
            ("2pi", "6,283185307"),
            ("2(3+4)", "14"),
            ("(1+2)(3+4)", "21"),
            ("sqrt 16", "4"),
            ("raiz(16) + 9", "13"),
            ("sin 30°", "0,5"),
            ("sen(90 graus)", "1"),
            ("cos(pi)", "-1"),
            ("sin(pi)", "0"),
            ("log 1000", "3"),
            ("ln e", "1"),
            ("abs(-3)", "3"),
            ("0,1 + 0,2", "0,3"),
            ("1/3", "0,3333333333"),
            ("1.500 * 2", "3.000"),
            ("1.5 * 2", "3"),
            ("1e3 + 1", "1.001"),
            ("10 × 3 ÷ 2", "15"),
            ("2^60", "1,152921505e18"),
        ],
    );
    check(
        Locale::En,
        &[
            ("1,000 + 1", "1,001"),
            ("1.5 + 1", "2.5"),
            ("1/3", "0.3333333333"),
            ("2^10", "1,024"),
        ],
    );
}

#[test]
fn percentages() {
    check(
        Locale::Pt,
        &[
            ("52% de 900", "468"),
            ("52% of 900", "468"),
            ("900 + 10%", "990"),
            ("900 - 10%", "810"),
            ("10% off 900", "810"),
            ("10% de desconto em 900", "810"),
            ("50% * 8", "4"),
            ("30/120 em %", "25%"),
            ("30/120 in %", "25%"),
        ],
    );
}

#[test]
fn units() {
    check(
        Locale::Pt,
        &[
            ("10 ft em m", "3,048 m"),
            ("10ft in m", "3,048 m"),
            ("5 ft 3 in em cm", "160,02 cm"),
            ("1 m + 20 cm", "1,2 m"),
            ("2 in em px", "192 px"),
            ("12 pt em px", "16 px"),
            ("1 rem em px", "16 px"),
            ("100 °C em °F", "212 °F"),
            ("-40 c in f", "-40 °F"),
            ("0 k em c", "-273,15 °C"),
            ("1 milha em km", "1,609344 km"),
            ("1 lb em kg", "0,45359237 kg"),
            ("1 gal em l", "3,785411784 l"),
            ("1 cup em ml", "236,5882365 ml"),
            ("1 GB em MB", "1.000 MB"),
            ("1 GiB em MiB", "1.024 MiB"),
            ("8 b em B", "1 B"),
            ("1 Mb em KB", "125 KB"),
            ("100 km/h em m/s", "27,77777778 m/s"),
            ("1 ha em m²", "10.000 m²"),
            ("2 h em min", "120 min"),
            ("90 min em h", "1,5 h"),
            ("1 ano em d", "365,2425 d"),
            ("1 sem em d", "7 d"),
            ("21 d em sem", "3 sem"),
            ("730,485 d em anos", "2 anos"),
            ("365,2425 d em ano", "1 ano"),
            ("180° em rad", "3,141592654 rad"),
            ("10 kg / 2", "5 kg"),
            ("10 m / 2 m", "5"),
            ("2 * 3 m", "6 m"),
        ],
    );
    check(
        Locale::En,
        &[
            ("10 ft in m", "3.048 m"),
            ("1 GB in MB", "1,000 MB"),
            ("21 d in wk", "3 wk"),
            ("730.485 d in years", "2 yr"),
        ],
    );
}

#[test]
fn durations() {
    check(
        Locale::Pt,
        &[
            ("145 min em duração", "2 h 25 min"),
            ("145 min in timespan", "2 h 25 min"),
            ("2h 30min + 45min", "3 h 15 min"),
            ("2h30min + 45min", "3 h 15 min"),
            ("90 s + 1 min", "2 min 30 s"),
            ("1,5 h em duracao", "1 h 30 min"),
            ("100000 s em duração", "1 d 3 h 46 min 40 s"),
            ("30 s em duração", "30 s"),
            ("2 h + 3 h", "5 h"),
        ],
    );
}

#[test]
fn answer_parts() {
    let pt = Context { locale: Locale::Pt };
    let answer = evaluate("1000 * 2", &pt).unwrap();
    assert_eq!(
        (answer.result.as_str(), answer.copy.as_str()),
        ("2.000", "2000")
    );
    assert_eq!(answer.kind, AnswerKind::Number);
    assert_eq!(answer.expression, "1000 × 2");
    let answer = evaluate("1 GB em MB", &pt).unwrap();
    assert_eq!(answer.copy, "1000 MB");
    assert_eq!(answer.kind, AnswerKind::Quantity);
    assert_eq!(evaluate("10ft in m", &pt).unwrap().expression, "10ft in m");
    assert_eq!(evaluate("10 ft -> m", &pt).unwrap().expression, "10 ft → m");
    assert_eq!(evaluate("2 ** 3", &pt).unwrap().expression, "2 ^ 3");
    assert_eq!(evaluate("8/2", &pt).unwrap().expression, "8÷2");
    assert_eq!(
        evaluate("145 min em duração", &pt).unwrap().kind,
        AnswerKind::Duration
    );
    let en = Context { locale: Locale::En };
    let answer = evaluate("1000*1.5", &en).unwrap();
    assert_eq!(
        (answer.result.as_str(), answer.copy.as_str()),
        ("1,500", "1500")
    );
}

#[test]
fn not_calculations() {
    let queries = [
        "",
        "   ",
        "2024",
        "relatório 2024",
        "notes.txt",
        "c:\\users",
        "v0.4.3",
        "1.2.3",
        "e",
        "pi",
        "m",
        "5 m",
        "31 mar",
        "2024-10-19",
        "rust-lang",
        "a-b",
        "2024-report",
        "c++",
        "tex em casa",
        "10 em m",
        "100 °C + 10 °C",
        "5 m + 3 kg",
        "1/0",
        "sqrt(-1)",
        "2 3",
        "-5",
        "50%",
        "30°",
        "km/h",
        "10 mod 0",
        "1 GB",
        "171!",
        "2,5!",
        "90 min em graus",
    ];
    for locale in [Locale::Pt, Locale::En] {
        for query in queries {
            assert_eq!(result(query, locale), None, "{query} ({locale:?})");
        }
    }
}
