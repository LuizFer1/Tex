mod dates;
mod eval;
mod format;
mod lexer;
mod parser;
mod units;
mod zones;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    Pt,
    En,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub locale: Locale,
    pub now: jiff::Zoned,
}

impl Context {
    /// The context of a query typed now, in the system's time zone.
    pub fn now(locale: Locale) -> Context {
        Context {
            locale,
            now: jiff::Zoned::now(),
        }
    }
}

/// The card's content: the query as shown, the result as shown, and the text to copy
/// (the result without the thousands separator).
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub expression: String,
    pub result: String,
    pub copy: String,
    pub kind: AnswerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKind {
    Number,
    Quantity,
    Duration,
}

/// The calculator's answer to a search query, or `None` when the query is not a complete
/// calculation that does something (a lone number, quantity or constant is not).
pub fn evaluate(query: &str, context: &Context) -> Option<Answer> {
    let tokens = lexer::tokens(query, context.locale)?;
    let parsed = parser::parse(&tokens)?;
    if !parsed.acts {
        return None;
    }
    let outcome = eval::run(&parsed)?;
    Some(format::answer(&tokens, outcome, context.locale))
}
