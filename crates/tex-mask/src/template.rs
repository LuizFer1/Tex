use regex_syntax::ParserBuilder;
use regex_syntax::hir::{Class, Hir, HirKind, Look};

const MAX_GROUP: usize = 64;

/// Why a pattern cannot be used as a mask. `code` is what the settings window turns into text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskError {
    Syntax,
    NotFixed,
    NoGroups,
    NoSeparator,
    TooLong,
}

impl MaskError {
    pub fn code(self) -> i32 {
        match self {
            MaskError::Syntax => 1,
            MaskError::NotFixed => 2,
            MaskError::NoGroups => 3,
            MaskError::NoSeparator => 4,
            MaskError::TooLong => 5,
        }
    }
}

/// Inclusive character ranges of one group. ASCII letters match in either case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharClass(Vec<(char, char)>);

impl CharClass {
    pub fn contains(&self, c: char) -> bool {
        let hit = |c: char| self.0.iter().any(|&(low, high)| low <= c && c <= high);
        hit(c) || hit(c.to_ascii_lowercase()) || hit(c.to_ascii_uppercase())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Literal(String),
    Group { class: CharClass, len: usize },
}

/// A mask as a sequence of separators and fixed-size groups, read from the regex's syntax tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    pub fn parse(pattern: &str) -> Result<Template, MaskError> {
        let hir = ParserBuilder::new()
            .unicode(false)
            .build()
            .parse(pattern)
            .map_err(|_| MaskError::Syntax)?;
        let mut items: Vec<&Hir> = match hir.kind() {
            HirKind::Concat(items) => items.iter().collect(),
            _ => vec![&hir],
        };
        if matches!(
            items.first().map(|item| item.kind()),
            Some(HirKind::Look(Look::Start))
        ) {
            items.remove(0);
        }
        if matches!(
            items.last().map(|item| item.kind()),
            Some(HirKind::Look(Look::End))
        ) {
            items.pop();
        }
        let mut parts = Vec::new();
        for item in items {
            match item.kind() {
                HirKind::Literal(literal) => {
                    let text = std::str::from_utf8(&literal.0).map_err(|_| MaskError::Syntax)?;
                    parts.push(Part::Literal(text.to_owned()));
                }
                HirKind::Capture(capture) => parts.push(group(&capture.sub)?),
                _ => return Err(MaskError::NotFixed),
            }
        }
        if !parts.iter().any(|part| matches!(part, Part::Group { .. })) {
            return Err(MaskError::NoGroups);
        }
        if !parts
            .iter()
            .any(|part| matches!(part, Part::Literal(text) if !text.is_empty()))
        {
            return Err(MaskError::NoSeparator);
        }
        Ok(Template { parts })
    }

    /// How many letters and digits the groups hold together.
    pub fn payload_len(&self) -> usize {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Group { len, .. } => *len,
                Part::Literal(_) => 0,
            })
            .sum()
    }

    /// Puts the mask on one line: every letter and digit of it, separators dropped, must fill the
    /// groups exactly. The line is returned masked, or `None` when it does not fit.
    pub fn format(&self, line: &str) -> Option<String> {
        let payload: Vec<char> = line.chars().filter(|c| c.is_alphanumeric()).collect();
        if payload.len() != self.payload_len() {
            return None;
        }
        let mut next = payload.iter();
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => out.push_str(text),
                Part::Group { class, len } => {
                    for _ in 0..*len {
                        let c = *next.next()?;
                        if !class.contains(c) {
                            return None;
                        }
                        out.push(c);
                    }
                }
            }
        }
        Some(out)
    }

    /// Reads the masked form starting exactly at `start`: returns where it ends and the groups
    /// joined without the separators.
    pub fn match_at(&self, chars: &[char], start: usize) -> Option<(usize, String)> {
        let mut position = start;
        let mut payload = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(text) => {
                    for expected in text.chars() {
                        let c = *chars.get(position)?;
                        if !c.eq_ignore_ascii_case(&expected) {
                            return None;
                        }
                        position += 1;
                    }
                }
                Part::Group { class, len } => {
                    for _ in 0..*len {
                        let c = *chars.get(position)?;
                        if !class.contains(c) {
                            return None;
                        }
                        payload.push(c);
                        position += 1;
                    }
                }
            }
        }
        Some((position, payload))
    }
}

fn group(sub: &Hir) -> Result<Part, MaskError> {
    let (inner, len) = match sub.kind() {
        HirKind::Repetition(repetition) if Some(repetition.min) == repetition.max => {
            (&*repetition.sub, repetition.min as usize)
        }
        _ => (sub, 1),
    };
    let HirKind::Class(class) = inner.kind() else {
        return Err(MaskError::NotFixed);
    };
    if len == 0 {
        return Err(MaskError::NotFixed);
    }
    if len > MAX_GROUP {
        return Err(MaskError::TooLong);
    }
    let ranges = match class {
        Class::Unicode(set) => set.iter().map(|r| (r.start(), r.end())).collect(),
        Class::Bytes(set) => set
            .iter()
            .filter(|r| r.start() < 0x80)
            .map(|r| (r.start() as char, r.end().min(0x7F) as char))
            .collect(),
    };
    Ok(Part::Group {
        class: CharClass(ranges),
        len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CNPJ: &str = r"([0-9A-Z]{2})\.([0-9A-Z]{3})\.([0-9A-Z]{3})/([0-9A-Z]{4})-(\d{2})";

    #[test]
    fn formats_digits_into_the_groups() {
        let template = Template::parse(CNPJ).unwrap();
        assert_eq!(template.payload_len(), 14);
        assert_eq!(
            template.format("12345678000195").as_deref(),
            Some("12.345.678/0001-95")
        );
    }

    #[test]
    fn accepts_letters_in_any_case_where_the_class_allows() {
        let template = Template::parse(CNPJ).unwrap();
        assert_eq!(
            template.format("12ABC345000195").as_deref(),
            Some("12.ABC.345/0001-95")
        );
        assert_eq!(
            template.format("12abc345000195").as_deref(),
            Some("12.abc.345/0001-95")
        );
    }

    #[test]
    fn refuses_a_wrong_length_or_class() {
        let template = Template::parse(CNPJ).unwrap();
        assert_eq!(template.format("1234567800019"), None);
        assert_eq!(template.format("1234567800019A"), None);
    }

    #[test]
    fn reformats_a_number_that_is_already_masked() {
        let template = Template::parse(r"(\d{3})\.(\d{3})\.(\d{3})-(\d{2})").unwrap();
        assert_eq!(
            template.format("123-456-789/09").as_deref(),
            Some("123.456.789-09")
        );
    }

    #[test]
    fn anchors_at_the_ends_are_ignored() {
        let template = Template::parse(r"^(\d{5})-(\d{3})$").unwrap();
        assert_eq!(template.format("01310100").as_deref(), Some("01310-100"));
    }

    #[test]
    fn refuses_what_has_no_fixed_size() {
        assert_eq!(Template::parse(r"(\d+)-(\d{2})"), Err(MaskError::NotFixed));
        assert_eq!(Template::parse(r"\d+"), Err(MaskError::NotFixed));
        assert_eq!(
            Template::parse(r"(\d{2})|(\d{3})"),
            Err(MaskError::NotFixed)
        );
        assert_eq!(Template::parse(r"(\d{2,3})-(\d)"), Err(MaskError::NotFixed));
        assert_eq!(Template::parse(r"((\d{2}))-(\d)"), Err(MaskError::NotFixed));
    }

    #[test]
    fn refuses_broken_or_useless_patterns() {
        assert_eq!(Template::parse("("), Err(MaskError::Syntax));
        assert_eq!(Template::parse("abc"), Err(MaskError::NoGroups));
        assert_eq!(
            Template::parse(r"(\d{3})(\d{3})"),
            Err(MaskError::NoSeparator)
        );
        assert_eq!(Template::parse(r"(\d{99})-(\d)"), Err(MaskError::TooLong));
    }

    #[test]
    fn matches_at_a_position_and_returns_the_groups_joined() {
        let template = Template::parse(r"\((\d{2})\) (\d{4})-(\d{4})").unwrap();
        let chars: Vec<char> = "tel (11) 3456-7890 ok".chars().collect();
        assert_eq!(
            template.match_at(&chars, 4),
            Some((18, "1134567890".to_owned()))
        );
        assert_eq!(template.match_at(&chars, 5), None);
    }
}
