use crate::validate_name;
use regex_syntax::ParserBuilder;
use regex_syntax::ast::parse::Parser as AstParser;
use regex_syntax::ast::{Ast, RepetitionKind, RepetitionRange};
use regex_syntax::hir::{Class, Hir, HirKind, Look};

pub const MAX_VALUE_LEN: usize = 256;
pub const SYMBOLS: &str = "!@#$%&*-_=+?";

const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const CNPJ_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const MIN_PASSWORD: usize = 4;
const MAX_PASSWORD: usize = 128;
const FIRST_PRINTABLE: u32 = 0x21;
const LAST_PRINTABLE: u32 = 0x7E;
const CPF_FIRST: [u32; 9] = [10, 9, 8, 7, 6, 5, 4, 3, 2];
const CPF_SECOND: [u32; 10] = [11, 10, 9, 8, 7, 6, 5, 4, 3, 2];
const CNPJ_FIRST: [u32; 12] = [5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
const CNPJ_SECOND: [u32; 13] = [6, 5, 4, 3, 2, 9, 8, 7, 6, 5, 4, 3, 2];
const CPF_MARKS: [(usize, char); 3] = [(3, '.'), (6, '.'), (9, '-')];
const CNPJ_MARKS: [(usize, char); 4] = [(2, '.'), (5, '.'), (8, '/'), (12, '-')];

/// Source of random numbers. `None` means the system source failed.
pub trait Random {
    /// A uniform number in `0..n` (n > 0).
    fn below(&mut self, n: u32) -> Option<u32>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Cpf,
    Cnpj,
    CnpjAlpha,
    Password,
}

impl Builtin {
    pub const ALL: [Builtin; 4] = [
        Builtin::Cpf,
        Builtin::Cnpj,
        Builtin::CnpjAlpha,
        Builtin::Password,
    ];

    /// The value saved in the settings.
    pub fn key(self) -> &'static str {
        match self {
            Builtin::Cpf => "cpf",
            Builtin::Cnpj => "cnpj",
            Builtin::CnpjAlpha => "cnpja",
            Builtin::Password => "password",
        }
    }

    pub fn from_key(key: &str) -> Option<Builtin> {
        Builtin::ALL
            .into_iter()
            .find(|builtin| builtin.key() == key)
    }
}

/// Why a pattern cannot generate values. The codes match `MaskError` where they overlap; 4 is
/// unused on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenError {
    Syntax,
    NotFixed,
    NoRandom,
    TooLong,
    NoPrintable,
}

impl GenError {
    pub fn code(self) -> i32 {
        match self {
            GenError::Syntax => 1,
            GenError::NotFixed => 2,
            GenError::NoRandom => 3,
            GenError::TooLong => 5,
            GenError::NoPrintable => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Literal(String),
    Random { chars: Vec<char>, len: usize },
}

/// A regex with a fixed layout, read into literal text and random characters from fixed sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pieces: Vec<Piece>,
}

impl Pattern {
    pub fn parse(pattern: &str) -> Result<Pattern, GenError> {
        let hir = ParserBuilder::new()
            .unicode(false)
            .utf8(false)
            .build()
            .parse(pattern)
            .map_err(|_| GenError::Syntax)?;
        let ast = AstParser::new()
            .parse(pattern)
            .map_err(|_| GenError::Syntax)?;
        check_ast(&ast)?;
        let mut builder = Builder {
            pieces: Vec::new(),
            length: 0,
        };
        builder.walk(&hir)?;
        if !builder
            .pieces
            .iter()
            .any(|piece| matches!(piece, Piece::Random { .. }))
        {
            return Err(GenError::NoRandom);
        }
        Ok(Pattern {
            pieces: builder.pieces,
        })
    }

    pub fn generate(&self, rng: &mut dyn Random) -> Option<String> {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Literal(text) => out.push_str(text),
                Piece::Random { chars, len } => {
                    for _ in 0..*len {
                        let index = rng.below(chars.len() as u32)?;
                        out.push(*chars.get(index as usize)?);
                    }
                }
            }
        }
        Some(out)
    }
}

struct Builder {
    pieces: Vec<Piece>,
    length: usize,
}

impl Builder {
    fn walk(&mut self, hir: &Hir) -> Result<(), GenError> {
        match hir.kind() {
            HirKind::Empty
            | HirKind::Look(
                Look::Start
                | Look::End
                | Look::StartLF
                | Look::EndLF
                | Look::StartCRLF
                | Look::EndCRLF,
            ) => Ok(()),
            HirKind::Literal(literal) => {
                let text = text_of(&literal.0)?;
                self.literal(text, 1)
            }
            HirKind::Capture(capture) => self.walk(&capture.sub),
            HirKind::Concat(items) => items.iter().try_for_each(|item| self.walk(item)),
            HirKind::Class(class) => self.random(class, 1),
            HirKind::Repetition(repetition) => {
                self.repeat(repetition.min, repetition.max, &repetition.sub)
            }
            _ => Err(GenError::NotFixed),
        }
    }

    fn repeat(&mut self, min: u32, max: Option<u32>, sub: &Hir) -> Result<(), GenError> {
        let count = min as usize;
        if max != Some(min) || count == 0 {
            return Err(GenError::NotFixed);
        }
        match sub.kind() {
            HirKind::Class(class) => self.random(class, count),
            HirKind::Literal(literal) => {
                let text = text_of(&literal.0)?;
                self.literal(text, count)
            }
            _ => Err(GenError::NotFixed),
        }
    }

    fn literal(&mut self, text: &str, count: usize) -> Result<(), GenError> {
        self.reserve(text.chars().count().saturating_mul(count))?;
        self.pieces.push(Piece::Literal(text.repeat(count)));
        Ok(())
    }

    fn random(&mut self, class: &Class, len: usize) -> Result<(), GenError> {
        let chars = printable(class);
        if chars.is_empty() {
            return Err(GenError::NoPrintable);
        }
        self.reserve(len)?;
        self.pieces.push(Piece::Random { chars, len });
        Ok(())
    }

    fn reserve(&mut self, chars: usize) -> Result<(), GenError> {
        self.length = self.length.saturating_add(chars);
        if self.length > MAX_VALUE_LEN {
            return Err(GenError::TooLong);
        }
        Ok(())
    }
}

fn text_of(bytes: &[u8]) -> Result<&str, GenError> {
    std::str::from_utf8(bytes).map_err(|_| GenError::Syntax)
}

/// The printable ASCII characters a class accepts, in ascending order.
fn printable(class: &Class) -> Vec<char> {
    let ranges: Vec<(u32, u32)> = match class {
        Class::Unicode(set) => set
            .iter()
            .map(|range| (range.start() as u32, range.end() as u32))
            .collect(),
        Class::Bytes(set) => set
            .iter()
            .map(|range| (u32::from(range.start()), u32::from(range.end())))
            .collect(),
    };
    let mut chars: Vec<char> = Vec::new();
    for (start, end) in ranges {
        let low = start.max(FIRST_PRINTABLE);
        let high = end.min(LAST_PRINTABLE);
        chars.extend((low..=high).filter_map(char::from_u32));
    }
    chars.sort_unstable();
    chars.dedup();
    chars
}

/// Refuses what the HIR hides: `a|b` is a class there, and `x{0}` disappears.
fn check_ast(ast: &Ast) -> Result<(), GenError> {
    match ast {
        Ast::Alternation(_) => Err(GenError::NotFixed),
        Ast::Concat(concat) => concat.asts.iter().try_for_each(check_ast),
        Ast::Group(group) => check_ast(&group.ast),
        Ast::Repetition(repetition) => match &repetition.op.kind {
            RepetitionKind::Range(RepetitionRange::Exactly(0) | RepetitionRange::Bounded(0, 0)) => {
                Err(GenError::NotFixed)
            }
            _ => check_ast(&repetition.ast),
        },
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenOptions {
    pub formatted: bool,
    pub password_length: usize,
    pub password_symbols: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenSource {
    Builtin(Builtin),
    Pattern(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenSpec {
    pub name: String,
    pub source: GenSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenKind {
    Builtin(Builtin),
    Pattern(Pattern),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generator {
    pub name: String,
    pub kind: GenKind,
}

impl Generator {
    /// One value. `None` when the random source failed.
    pub fn generate(&self, rng: &mut dyn Random, options: GenOptions) -> Option<String> {
        match &self.kind {
            GenKind::Builtin(Builtin::Cpf) => cpf(rng, options.formatted),
            GenKind::Builtin(Builtin::Cnpj) => cnpj(rng, options.formatted),
            GenKind::Builtin(Builtin::CnpjAlpha) => cnpj_alpha(rng, options.formatted),
            GenKind::Builtin(Builtin::Password) => {
                password(rng, options.password_length, options.password_symbols)
            }
            GenKind::Pattern(pattern) => pattern.generate(rng),
        }
    }
}

/// A random CPF, shaped `###.###.###-##` when formatted.
pub fn cpf(rng: &mut dyn Random, formatted: bool) -> Option<String> {
    let base = loop {
        let digits = random_chars(rng, DIGITS, 9)?;
        if !all_equal(&digits) {
            break digits;
        }
    };
    let check = cpf_check(&digit_values(&base));
    let mut chars = base;
    chars.extend(check.iter().map(|digit| digit_char(*digit)));
    let marks: &[(usize, char)] = if formatted { &CPF_MARKS } else { &[] };
    Some(with_marks(&chars, marks))
}

/// A random CNPJ with the `0001` branch, shaped `##.###.###/####-##` when formatted.
pub fn cnpj(rng: &mut dyn Random, formatted: bool) -> Option<String> {
    let mut base = loop {
        let digits = random_chars(rng, DIGITS, 8)?;
        if !all_equal(&digits) {
            break digits;
        }
    };
    base.extend("0001".chars());
    Some(cnpj_text(&base, formatted))
}

/// A random alphanumeric CNPJ, with at least one letter, shaped like `cnpj` when formatted.
pub fn cnpj_alpha(rng: &mut dyn Random, formatted: bool) -> Option<String> {
    let base = loop {
        let chars = random_chars(rng, CNPJ_CHARS, 12)?;
        if chars.iter().any(char::is_ascii_uppercase) {
            break chars;
        }
    };
    Some(cnpj_text(&base, formatted))
}

/// A random password of 4 to 128 characters, with one of each set placed first and then shuffled.
pub fn password(rng: &mut dyn Random, length: usize, symbols: bool) -> Option<String> {
    let length = length.clamp(MIN_PASSWORD, MAX_PASSWORD);
    let mut sets = vec![UPPER, LOWER, DIGITS];
    if symbols {
        sets.push(SYMBOLS);
    }
    let union = sets.concat();
    let mut chars: Vec<char> = Vec::with_capacity(length);
    for set in &sets {
        chars.push(pick(rng, set)?);
    }
    while chars.len() < length {
        chars.push(pick(rng, &union)?);
    }
    for index in (1..chars.len()).rev() {
        let other = rng.below(index as u32 + 1)? as usize;
        chars.swap(index, other);
    }
    Some(chars.into_iter().collect())
}

/// Whether the text is a valid CPF. Anything that is not a letter or digit is ignored.
pub fn cpf_valid(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    if chars.len() != 11 || !chars.iter().all(char::is_ascii_digit) {
        return false;
    }
    let values = digit_values(&chars);
    if all_equal(&values) {
        return false;
    }
    let check = cpf_check(&values[..9]);
    values[9] == check[0] && values[10] == check[1]
}

/// Whether the text is a valid CNPJ, numeric or alphanumeric. Case and punctuation are ignored.
pub fn cnpj_valid(text: &str) -> bool {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if chars.len() != 14
        || !chars[..12]
            .iter()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        || !chars[12..].iter().all(char::is_ascii_digit)
    {
        return false;
    }
    let check = cnpj_check(&digit_values(&chars[..12]));
    chars[12] == digit_char(check[0]) && chars[13] == digit_char(check[1])
}

/// The generators Tex starts with: CPF, CNPJ, alphanumeric CNPJ and a password named for the
/// interface language.
pub fn default_generators(english: bool) -> Vec<GenSpec> {
    let password = if english { "password" } else { "senha" };
    [
        ("cpf", Builtin::Cpf),
        ("cnpj", Builtin::Cnpj),
        ("cnpja", Builtin::CnpjAlpha),
        (password, Builtin::Password),
    ]
    .into_iter()
    .map(|(name, builtin)| GenSpec {
        name: name.to_owned(),
        source: GenSource::Builtin(builtin),
    })
    .collect()
}

/// Keeps the generators that can be used, in order. A name already taken by an earlier one, a
/// name that is not one word, or a pattern that does not parse, is left out.
pub fn compile_generators(specs: &[GenSpec]) -> Vec<Generator> {
    let mut compiled: Vec<Generator> = Vec::new();
    for spec in specs {
        let taken: Vec<&str> = compiled
            .iter()
            .map(|generator| generator.name.as_str())
            .collect();
        if validate_name(&spec.name, &taken).is_err() {
            continue;
        }
        let kind = match &spec.source {
            GenSource::Builtin(builtin) => GenKind::Builtin(*builtin),
            GenSource::Pattern(text) => match Pattern::parse(text) {
                Ok(pattern) => GenKind::Pattern(pattern),
                Err(_) => continue,
            },
        };
        compiled.push(Generator {
            name: spec.name.clone(),
            kind,
        });
    }
    compiled
}

fn cnpj_text(base: &[char], formatted: bool) -> String {
    let check = cnpj_check(&digit_values(base));
    let mut chars = base.to_vec();
    chars.extend(check.iter().map(|digit| digit_char(*digit)));
    let marks: &[(usize, char)] = if formatted { &CNPJ_MARKS } else { &[] };
    with_marks(&chars, marks)
}

fn cpf_check(base: &[u32]) -> [u32; 2] {
    check_digits(base, &CPF_FIRST, &CPF_SECOND)
}

fn cnpj_check(base: &[u32]) -> [u32; 2] {
    check_digits(base, &CNPJ_FIRST, &CNPJ_SECOND)
}

fn check_digits(base: &[u32], first: &[u32], second: &[u32]) -> [u32; 2] {
    let first_digit = check_digit(weighted(base, first));
    let extended: Vec<u32> = base.iter().copied().chain([first_digit]).collect();
    [first_digit, check_digit(weighted(&extended, second))]
}

fn weighted(values: &[u32], weights: &[u32]) -> u32 {
    values
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * weight)
        .sum()
}

fn check_digit(sum: u32) -> u32 {
    let rest = sum % 11;
    if rest < 2 { 0 } else { 11 - rest }
}

fn digit_values(chars: &[char]) -> Vec<u32> {
    chars.iter().map(|c| *c as u32 - 48).collect()
}

fn digit_char(value: u32) -> char {
    char::from(b'0' + value as u8)
}

fn all_equal<T: PartialEq>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] == pair[1])
}

fn with_marks(chars: &[char], marks: &[(usize, char)]) -> String {
    let mut out = String::with_capacity(chars.len() + marks.len());
    for (index, c) in chars.iter().enumerate() {
        for (at, mark) in marks {
            if *at == index {
                out.push(*mark);
            }
        }
        out.push(*c);
    }
    out
}

fn random_chars(rng: &mut dyn Random, set: &str, count: usize) -> Option<Vec<char>> {
    let mut chars = Vec::with_capacity(count);
    for _ in 0..count {
        chars.push(pick(rng, set)?);
    }
    Some(chars)
}

fn pick(rng: &mut dyn Random, set: &str) -> Option<char> {
    let index = rng.below(set.len() as u32)?;
    set.as_bytes()
        .get(index as usize)
        .map(|byte| char::from(*byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seeded(u64);

    impl Random for Seeded {
        fn below(&mut self, n: u32) -> Option<u32> {
            let span = 1u64 << 32;
            let limit = span - span % u64::from(n);
            loop {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                let draw = self.0 >> 32;
                if draw < limit {
                    return Some((draw % u64::from(n)) as u32);
                }
            }
        }
    }

    struct Scripted(Vec<u32>);

    impl Random for Scripted {
        fn below(&mut self, n: u32) -> Option<u32> {
            if self.0.is_empty() {
                return None;
            }
            let value = self.0.remove(0);
            (value < n).then_some(value)
        }
    }

    struct Failing;

    impl Random for Failing {
        fn below(&mut self, _n: u32) -> Option<u32> {
            None
        }
    }

    fn seeded() -> Seeded {
        Seeded(0x9E37_79B9_7F4A_7C15)
    }

    fn has_shape(text: &str, shape: &str) -> bool {
        text.chars().count() == shape.chars().count()
            && text
                .chars()
                .zip(shape.chars())
                .all(|(value, slot)| match slot {
                    '#' => value.is_ascii_digit(),
                    '@' => value.is_ascii_digit() || value.is_ascii_uppercase(),
                    other => value == other,
                })
    }

    #[test]
    fn check_digits_match_the_official_examples() {
        assert_eq!(cpf_check(&[1, 2, 3, 4, 5, 6, 7, 8, 9]), [0, 9]);
        assert_eq!(cnpj_check(&[1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 1]), [9, 5]);
        let alpha: Vec<u32> = "12ABC34501DE".chars().map(|c| c as u32 - 48).collect();
        assert_eq!(cnpj_check(&alpha), [3, 5]);
    }

    #[test]
    fn a_scripted_all_equal_draw_is_drawn_again() {
        let mut rng = Scripted(vec![3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(cpf(&mut rng, true).as_deref(), Some("123.456.789-09"));
        assert!(rng.0.is_empty());

        let mut rng = Scripted(vec![7, 7, 7, 7, 7, 7, 7, 7, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(cnpj(&mut rng, true).as_deref(), Some("12.345.678/0001-95"));
        assert!(rng.0.is_empty());
    }

    #[test]
    fn a_scripted_draw_without_letters_is_drawn_again_for_cnpj_alpha() {
        let mut rng = Scripted(vec![
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
        ]);
        let text = cnpj_alpha(&mut rng, false).unwrap();
        assert!(rng.0.is_empty());
        assert!(text.starts_with("ABCDEFGHIJKL"), "{text}");
        assert_eq!(text.len(), 14);
        assert!(cnpj_valid(&text));
    }

    #[test]
    fn generated_documents_pass_the_check_digits_in_every_shape() {
        let mut rng = seeded();
        for _ in 0..500 {
            let plain = cpf(&mut rng, false).unwrap();
            assert!(
                has_shape(&plain, &"#".repeat(11)) && cpf_valid(&plain),
                "{plain}"
            );
            let shaped = cpf(&mut rng, true).unwrap();
            assert!(
                has_shape(&shaped, "###.###.###-##") && cpf_valid(&shaped),
                "{shaped}"
            );

            let plain = cnpj(&mut rng, false).unwrap();
            assert!(
                has_shape(&plain, &"#".repeat(14)) && cnpj_valid(&plain),
                "{plain}"
            );
            assert_eq!(&plain[8..12], "0001", "{plain}");
            let shaped = cnpj(&mut rng, true).unwrap();
            assert!(
                has_shape(&shaped, "##.###.###/####-##") && cnpj_valid(&shaped),
                "{shaped}"
            );

            let plain = cnpj_alpha(&mut rng, false).unwrap();
            assert!(
                has_shape(&plain, &("@".repeat(12) + "##")) && cnpj_valid(&plain),
                "{plain}"
            );
            let shaped = cnpj_alpha(&mut rng, true).unwrap();
            assert!(
                has_shape(&shaped, "@@.@@@.@@@/@@@@-##") && cnpj_valid(&shaped),
                "{shaped}"
            );
        }
    }

    #[test]
    fn cnpj_alpha_always_has_a_letter() {
        let mut rng = seeded();
        for _ in 0..500 {
            let text = cnpj_alpha(&mut rng, false).unwrap();
            assert!(text.chars().any(|c| c.is_ascii_uppercase()), "{text}");
        }
    }

    #[test]
    fn passwords_are_clamped_and_hold_each_set() {
        let mut rng = seeded();
        let short = password(&mut rng, 2, true).unwrap();
        assert_eq!(short.chars().count(), 4);
        for set in [UPPER, LOWER, DIGITS, SYMBOLS] {
            assert_eq!(
                short.chars().filter(|c| set.contains(*c)).count(),
                1,
                "{short}"
            );
        }

        let long = password(&mut rng, 500, true).unwrap();
        assert_eq!(long.chars().count(), 128);
        for set in [UPPER, LOWER, DIGITS, SYMBOLS] {
            assert!(long.chars().any(|c| set.contains(c)), "{long}");
        }

        let plain = password(&mut rng, 20, false).unwrap();
        assert_eq!(plain.chars().count(), 20);
        assert!(plain.chars().all(|c| !SYMBOLS.contains(c)), "{plain}");
    }

    #[test]
    fn patterns_with_a_fixed_layout_parse() {
        for pattern in [
            r"[A-Z]{3}\d[A-Z]\d{2}",
            r"^(\d{3})\.(\d{3})$",
            "TEX-[0-9A-F]{8}",
            r"a{3}\d",
        ] {
            assert!(Pattern::parse(pattern).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn patterns_without_a_fixed_layout_fail_with_their_code() {
        let cases = [
            ("(", GenError::Syntax),
            (r"\d+", GenError::NotFixed),
            ("a|b", GenError::NotFixed),
            (r"\d|x", GenError::NotFixed),
            (r"\d?", GenError::NotFixed),
            (r"\d{2,3}", GenError::NotFixed),
            (r"\d\d{0}", GenError::NotFixed),
            (r"\bA\d", GenError::NotFixed),
            ("abc", GenError::NoRandom),
            (r"\d{300}", GenError::TooLong),
            (r"[\x00-\x1F]", GenError::NoPrintable),
        ];
        for (pattern, error) in cases {
            assert_eq!(Pattern::parse(pattern), Err(error), "{pattern}");
        }
    }

    #[test]
    fn the_length_limit_is_checked_before_anything_is_built() {
        assert!(Pattern::parse(r"\d{256}").is_ok());
        assert_eq!(Pattern::parse(r"\d{257}"), Err(GenError::TooLong));
        assert_eq!(Pattern::parse(r"a{4000000000}\d"), Err(GenError::TooLong));
    }

    #[test]
    fn the_mercosul_plate_has_seven_characters_in_place() {
        let plate = Pattern::parse(r"[A-Z]{3}\d[A-Z]\d{2}").unwrap();
        let mut rng = seeded();
        for _ in 0..200 {
            let text: Vec<char> = plate.generate(&mut rng).unwrap().chars().collect();
            assert_eq!(text.len(), 7);
            assert!(text[..3].iter().all(char::is_ascii_uppercase));
            assert!(text[3].is_ascii_digit());
            assert!(text[4].is_ascii_uppercase());
            assert!(text[5..].iter().all(char::is_ascii_digit));
        }
    }

    #[test]
    fn pattern_values_keep_the_literal_text_and_the_classes() {
        let mut rng = seeded();
        let masked = Pattern::parse(r"^(\d{3})\.(\d{3})$").unwrap();
        let text = masked.generate(&mut rng).unwrap();
        assert!(has_shape(&text, "###.###"), "{text}");

        let tex = Pattern::parse("TEX-[0-9A-F]{8}").unwrap();
        let text = tex.generate(&mut rng).unwrap();
        assert_eq!(text.len(), 12);
        assert!(text.starts_with("TEX-"), "{text}");
        assert!(
            text[4..]
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()),
            "{text}"
        );

        let repeated = Pattern::parse(r"a{3}\d").unwrap();
        let text = repeated.generate(&mut rng).unwrap();
        assert!(text.starts_with("aaa") && text.len() == 4, "{text}");
        assert!(text.ends_with(|c: char| c.is_ascii_digit()), "{text}");
    }

    #[test]
    fn a_dot_gives_only_printable_characters() {
        let any = Pattern::parse(".{200}").unwrap();
        let text = any.generate(&mut seeded()).unwrap();
        assert_eq!(text.chars().count(), 200);
        assert!(text.chars().all(|c| ('!'..='~').contains(&c)), "{text}");
    }

    #[test]
    fn a_failing_random_source_gives_none_everywhere() {
        let mut rng = Failing;
        let options = GenOptions {
            formatted: true,
            password_length: 12,
            password_symbols: true,
        };
        assert_eq!(cpf(&mut rng, true), None);
        assert_eq!(cnpj(&mut rng, true), None);
        assert_eq!(cnpj_alpha(&mut rng, true), None);
        assert_eq!(password(&mut rng, 8, false), None);
        assert_eq!(Pattern::parse(r"\d{3}").unwrap().generate(&mut rng), None);
        for generator in compile_generators(&default_generators(false)) {
            assert_eq!(
                generator.generate(&mut rng, options),
                None,
                "{}",
                generator.name
            );
        }
    }

    #[test]
    fn generators_use_the_options_of_their_kind() {
        let mut rng = seeded();
        let options = GenOptions {
            formatted: true,
            password_length: 10,
            password_symbols: false,
        };
        let generators = compile_generators(&default_generators(true));
        let shapes = ["###.###.###-##", "##.###.###/####-##", "@@.@@@.@@@/@@@@-##"];
        for (generator, shape) in generators.iter().zip(shapes) {
            let text = generator.generate(&mut rng, options).unwrap();
            assert!(has_shape(&text, shape), "{text}");
        }
        let text = generators[3].generate(&mut rng, options).unwrap();
        assert_eq!(text.chars().count(), 10);
        assert!(text.chars().all(|c| !SYMBOLS.contains(c)), "{text}");
    }

    #[test]
    fn default_generators_are_named_for_the_language() {
        let names = |english: bool| -> Vec<String> {
            default_generators(english)
                .into_iter()
                .map(|spec| spec.name)
                .collect()
        };
        assert_eq!(names(false), vec!["cpf", "cnpj", "cnpja", "senha"]);
        assert_eq!(names(true).last().map(String::as_str), Some("password"));
        assert_eq!(
            default_generators(false)[3].source,
            GenSource::Builtin(Builtin::Password)
        );
    }

    #[test]
    fn compile_skips_duplicates_bad_patterns_and_bad_names() {
        let specs = vec![
            GenSpec {
                name: "cpf".into(),
                source: GenSource::Builtin(Builtin::Cpf),
            },
            GenSpec {
                name: "CPF".into(),
                source: GenSource::Builtin(Builtin::Cpf),
            },
            GenSpec {
                name: "meu codigo".into(),
                source: GenSource::Pattern(r"\d{3}".into()),
            },
            GenSpec {
                name: "ruim".into(),
                source: GenSource::Pattern(r"\d+".into()),
            },
            GenSpec {
                name: "código".into(),
                source: GenSource::Pattern(r"\d{3}".into()),
            },
            GenSpec {
                name: "CÓDIGO".into(),
                source: GenSource::Pattern(r"\d{3}".into()),
            },
            GenSpec {
                name: "ok".into(),
                source: GenSource::Pattern(r"(\d{2})-(\d{2})".into()),
            },
        ];
        let compiled = compile_generators(&specs);
        let names: Vec<&str> = compiled
            .iter()
            .map(|generator| generator.name.as_str())
            .collect();
        assert_eq!(names, vec!["cpf", "código", "ok"]);
    }

    #[test]
    fn builtin_keys_round_trip() {
        for builtin in Builtin::ALL {
            assert_eq!(Builtin::from_key(builtin.key()), Some(builtin));
        }
        assert_eq!(Builtin::from_key("CPF"), None);
        assert_eq!(Builtin::from_key("senha"), None);
        assert_eq!(Builtin::from_key(""), None);
    }

    #[test]
    fn error_codes_line_up_with_mask_error() {
        let codes = [
            GenError::Syntax,
            GenError::NotFixed,
            GenError::NoRandom,
            GenError::TooLong,
            GenError::NoPrintable,
        ]
        .map(GenError::code);
        assert_eq!(codes, [1, 2, 3, 5, 6]);
    }

    #[test]
    fn validators_accept_the_examples_and_refuse_the_bad_ones() {
        assert!(cpf_valid("123.456.789-09"));
        assert!(cnpj_valid("12.345.678/0001-95"));
        assert!(cnpj_valid("12.ABC.345/01DE-35"));
        assert!(cnpj_valid("12abc34501de35"));
        assert!(!cpf_valid("111.111.111-11"));
        assert!(!cpf_valid("123.456.789-0"));
        assert!(!cnpj_valid("12.345.678/0001-96"));
        assert!(!cnpj_valid("12.ABC.345/01DE-3"));
    }
}
