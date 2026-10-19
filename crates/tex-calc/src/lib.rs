mod lexer;
mod parser;
mod units;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    Pt,
    En,
}
