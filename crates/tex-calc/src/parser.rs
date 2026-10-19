use std::f64::consts::{E, PI, TAU};

use crate::lexer::{Tok, Token};
use crate::units::{self, Dim, Unit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sqrt,
    Cbrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log,
    Log2,
    Abs,
    Round,
    Floor,
    Ceil,
    Exp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Qty(f64, &'static Unit),
    Percent(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
    Fact(Box<Expr>),
    PercentOf(Box<Expr>, Box<Expr>),
    Off(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Unit(&'static Unit),
    Percent,
    Timespan,
}

/// A parsed query. `acts` tells whether it does something (an operation, function, percentage
/// phrase or conversion); `time_units` counts the distinct time units it was written with.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub expr: Expr,
    pub target: Option<Target>,
    pub acts: bool,
    pub time_units: usize,
}

const FUNCTIONS: &[(&str, Func)] = &[
    ("sqrt", Func::Sqrt),
    ("raiz", Func::Sqrt),
    ("cbrt", Func::Cbrt),
    ("sin", Func::Sin),
    ("sen", Func::Sin),
    ("cos", Func::Cos),
    ("tan", Func::Tan),
    ("tg", Func::Tan),
    ("asin", Func::Asin),
    ("acos", Func::Acos),
    ("atan", Func::Atan),
    ("ln", Func::Ln),
    ("log", Func::Log),
    ("log2", Func::Log2),
    ("abs", Func::Abs),
    ("round", Func::Round),
    ("arred", Func::Round),
    ("floor", Func::Floor),
    ("piso", Func::Floor),
    ("ceil", Func::Ceil),
    ("teto", Func::Ceil),
    ("exp", Func::Exp),
];

const CONSTANTS: &[(&str, f64)] = &[("pi", PI), ("π", PI), ("e", E), ("tau", TAU), ("τ", TAU)];
const CONVERT: &[&str] = &["em", "in", "para", "to"];
const TIMESPAN: &[&str] = &["duracao", "timespan", "duration"];

pub fn parse(tokens: &[Token]) -> Option<Parsed> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        acts: false,
        time_units: Vec::new(),
    };
    let expr = parser.sum()?;
    let target = if parser.at_conversion() {
        parser.pos += 1;
        parser.acts = true;
        Some(parser.target()?)
    } else {
        None
    };
    if parser.pos != tokens.len() {
        return None;
    }
    Some(Parsed {
        expr,
        target,
        acts: parser.acts,
        time_units: parser.time_units.len(),
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    acts: bool,
    time_units: Vec<&'static str>,
}

impl Parser<'_> {
    fn peek(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.pos + ahead)
    }

    fn is_op(&self, op: char) -> bool {
        self.peek(0).is_some_and(|token| token.tok == Tok::Op(op))
    }

    fn is_word(&self, ahead: usize, words: &[&str]) -> bool {
        self.peek(ahead)
            .is_some_and(|token| token.tok == Tok::Word && words.contains(&token.folded.as_str()))
    }

    fn at_conversion(&self) -> bool {
        self.peek(0).is_some_and(|token| token.tok == Tok::Arrow) || self.is_word(0, CONVERT)
    }

    fn unit_at(&self, ahead: usize) -> Option<(&'static Unit, usize)> {
        let first = self.peek(ahead).filter(|token| token.tok == Tok::Word)?;
        if let Some(second) = self.peek(ahead + 1).filter(|token| token.tok == Tok::Word)
            && let Some(unit) = units::lookup(&format!("{} {}", first.text, second.text))
        {
            return Some((unit, 2));
        }
        units::lookup(&first.text).map(|unit| (unit, 1))
    }

    fn starts_target(&self, ahead: usize) -> bool {
        self.peek(ahead)
            .is_some_and(|token| token.tok == Tok::Op('%'))
            || self.is_word(ahead, TIMESPAN)
            || self.unit_at(ahead).is_some()
    }

    fn target(&mut self) -> Option<Target> {
        if self.is_op('%') {
            self.pos += 1;
            return Some(Target::Percent);
        }
        if self.is_word(0, TIMESPAN) {
            self.pos += 1;
            return Some(Target::Timespan);
        }
        let (unit, length) = self.unit_at(0)?;
        self.pos += length;
        Some(Target::Unit(unit))
    }

    /// The unit written after a number, if any. `in` and `em` followed by something that can
    /// be a conversion target are the conversion, not a unit (`5 ft in m`), unless another
    /// conversion follows them (`3 in in cm`).
    fn unit_after_number(&mut self) -> Option<&'static Unit> {
        if self.is_word(0, CONVERT)
            && self.starts_target(1)
            && !(self.is_word(1, CONVERT) && self.starts_target(2))
        {
            return None;
        }
        let (unit, length) = self.unit_at(0)?;
        self.pos += length;
        if unit.dim == Dim::Time && !self.time_units.contains(&unit.symbol) {
            self.time_units.push(unit.symbol);
        }
        Some(unit)
    }

    fn sum(&mut self) -> Option<Expr> {
        let mut left = self.product()?;
        loop {
            let op = if self.is_op('+') {
                BinOp::Add
            } else if self.is_op('-') {
                BinOp::Sub
            } else {
                break;
            };
            self.pos += 1;
            let right = self.product()?;
            self.acts = true;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn starts_implicit(&self) -> bool {
        let after_number = self.pos > 0 && matches!(self.tokens[self.pos - 1].tok, Tok::Num(_));
        self.is_op('(')
            || self.peek(0).is_some_and(|token| {
                token.tok == Tok::Word
                    && !(after_number && token.folded == "e")
                    && (CONSTANTS.iter().any(|(name, _)| *name == token.folded)
                        || FUNCTIONS.iter().any(|(name, _)| *name == token.folded))
            })
    }

    fn product(&mut self) -> Option<Expr> {
        let mut left = self.percent()?;
        loop {
            let op = if self.is_op('*') {
                BinOp::Mul
            } else if self.is_op('/') {
                BinOp::Div
            } else if self.is_word(0, &["mod"]) {
                BinOp::Mod
            } else if self.starts_implicit() {
                let right = self.percent()?;
                self.acts = true;
                left = Expr::Bin(BinOp::Mul, Box::new(left), Box::new(right));
                continue;
            } else {
                break;
            };
            self.pos += 1;
            let right = self.percent()?;
            self.acts = true;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn percent(&mut self) -> Option<Expr> {
        let left = self.unary()?;
        if !matches!(left, Expr::Percent(_)) {
            return Some(left);
        }
        let skip = if self.is_word(0, &["de"])
            && self.is_word(1, &["desconto"])
            && self.is_word(2, &["em", "sobre"])
        {
            Some((3, true))
        } else if self.is_word(0, &["off"]) {
            Some((1, true))
        } else if self.is_word(0, &["of", "de"]) {
            Some((1, false))
        } else {
            None
        };
        let Some((length, off)) = skip else {
            return Some(left);
        };
        self.pos += length;
        let base = self.unary()?;
        self.acts = true;
        Some(if off {
            Expr::Off(Box::new(left), Box::new(base))
        } else {
            Expr::PercentOf(Box::new(left), Box::new(base))
        })
    }

    fn unary(&mut self) -> Option<Expr> {
        if self.is_op('-') {
            self.pos += 1;
            return Some(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.is_op('+') {
            self.pos += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<Expr> {
        let base = self.postfix()?;
        if !self.is_op('^') {
            return Some(base);
        }
        self.pos += 1;
        let exponent = self.unary()?;
        self.acts = true;
        Some(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exponent)))
    }

    fn postfix(&mut self) -> Option<Expr> {
        let mut expr = self.primary()?;
        loop {
            if self.is_op('!') {
                self.pos += 1;
                self.acts = true;
                expr = Expr::Fact(Box::new(expr));
            } else if self.is_op('%') {
                self.pos += 1;
                expr = Expr::Percent(Box::new(expr));
            } else {
                return Some(expr);
            }
        }
    }

    fn primary(&mut self) -> Option<Expr> {
        let token = self.peek(0)?.clone();
        match token.tok {
            Tok::Num(value) => {
                self.pos += 1;
                self.quantity(value)
            }
            Tok::Op('(') => {
                self.pos += 1;
                let inner = self.sum()?;
                if !self.is_op(')') {
                    return None;
                }
                self.pos += 1;
                Some(inner)
            }
            Tok::Word => {
                if let Some(&(_, value)) = CONSTANTS.iter().find(|(name, _)| *name == token.folded)
                {
                    self.pos += 1;
                    return Some(Expr::Num(value));
                }
                let &(_, func) = FUNCTIONS.iter().find(|(name, _)| *name == token.folded)?;
                self.pos += 1;
                let arg = if self.is_op('(') {
                    self.acts = true;
                    self.primary()?
                } else {
                    self.power()?
                };
                Some(Expr::Call(func, Box::new(arg)))
            }
            _ => None,
        }
    }

    fn quantity(&mut self, value: f64) -> Option<Expr> {
        let Some(unit) = self.unit_after_number() else {
            return Some(Expr::Num(value));
        };
        let mut expr = Expr::Qty(value, unit);
        while let Some(Tok::Num(next)) = self.peek(0).map(|token| token.tok.clone()) {
            let save = self.pos;
            let units_before = self.time_units.len();
            self.pos += 1;
            match self.unit_after_number() {
                Some(next_unit) if next_unit.dim == unit.dim => {
                    self.acts = true;
                    expr = Expr::Bin(
                        BinOp::Add,
                        Box::new(expr),
                        Box::new(Expr::Qty(next, next_unit)),
                    );
                }
                _ => {
                    self.pos = save;
                    self.time_units.truncate(units_before);
                    break;
                }
            }
        }
        Some(expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Locale;
    use crate::lexer::{FUNCTION_NAMES, tokens};

    fn parsed(query: &str) -> Option<Parsed> {
        parse(&tokens(query, Locale::Pt)?)
    }

    fn acts(query: &str) -> bool {
        parsed(query).is_some_and(|parsed| parsed.acts)
    }

    #[test]
    fn function_tables_agree() {
        assert_eq!(FUNCTIONS.len(), FUNCTION_NAMES.len());
        for (name, _) in FUNCTIONS {
            assert!(FUNCTION_NAMES.contains(name), "{name}");
        }
    }

    #[test]
    fn only_queries_that_do_something_act() {
        for query in [
            "2+2",
            "2pi",
            "sqrt(16)",
            "sqrt 16 + 9",
            "5!",
            "10 ft em m",
            "5 ft 3 in",
            "52% de 900",
            "2^3",
            "30/120 em %",
        ] {
            assert!(acts(query), "{query}");
        }
        for query in [
            "2024", "-5", "50%", "pi", "e", "5 m", "30°", "sqrt 16", "ln e",
        ] {
            assert!(!acts(query), "{query}");
        }
    }

    #[test]
    fn incomplete_queries_do_not_parse() {
        for query in [
            "",
            "2 3",
            "m",
            "km/h",
            "(2+3",
            "2+",
            "rust-lang",
            "a-b",
            "2024-report",
            "c++",
            "tex em casa",
            "31 mar",
        ] {
            assert_eq!(parsed(query), None, "{query}");
        }
    }

    #[test]
    fn in_is_a_unit_or_a_conversion() {
        let target = |query: &str| parsed(query).and_then(|parsed| parsed.target);
        assert_eq!(
            target("5 ft in m"),
            Some(Target::Unit(units::lookup("m").unwrap()))
        );
        assert_eq!(
            target("3 in in cm"),
            Some(Target::Unit(units::lookup("cm").unwrap()))
        );
        assert_eq!(target("5 in"), None);
        assert!(acts("5 ft 3 in em cm"));
    }

    #[test]
    fn precedence() {
        let expr = |query: &str| parsed(query).unwrap().expr;
        assert_eq!(
            expr("-2^2"),
            Expr::Neg(Box::new(Expr::Bin(
                BinOp::Pow,
                Box::new(Expr::Num(2.0)),
                Box::new(Expr::Num(2.0))
            )))
        );
        assert!(matches!(expr("1+2*3"), Expr::Bin(BinOp::Add, _, _)));
        assert!(
            matches!(expr("2^3^2"), Expr::Bin(BinOp::Pow, _, right) if matches!(*right, Expr::Bin(BinOp::Pow, _, _)))
        );
        assert!(
            matches!(expr("sin(30)^2"), Expr::Bin(BinOp::Pow, left, _) if matches!(*left, Expr::Call(Func::Sin, _)))
        );
        assert!(matches!(expr("10% off 900"), Expr::Off(_, _)));
        assert!(matches!(expr("10% de desconto em 900"), Expr::Off(_, _)));
        assert!(matches!(expr("52% of 900"), Expr::PercentOf(_, _)));
    }

    #[test]
    fn time_units_are_counted() {
        assert_eq!(parsed("2h30min + 45min").unwrap().time_units, 2);
        assert_eq!(parsed("2 h + 3 h").unwrap().time_units, 1);
        assert_eq!(parsed("5 m").unwrap().time_units, 0);
    }
}
