use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};

use crate::Context;
use crate::dates::{self, YearRule};
use crate::parser::{BinOp, Expr, Func, Parsed, Target};
use crate::units::{self, Dim, Unit};

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Num(f64),
    Qty(f64, &'static Unit),
    Date(Date),
    Clock(Time),
    At(Zoned, String),
    Span(f64),
}

/// What a query evaluates to, ready to format. `Duration` is in seconds.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Number(f64),
    Percent(f64),
    Quantity(f64, &'static Unit),
    Duration(f64),
    Date(Date),
    Time {
        time: Time,
        place: String,
        weekday: u8,
        offset_minutes: i32,
    },
}

impl Outcome {
    fn is_finite(&self) -> bool {
        match self {
            Outcome::Number(x)
            | Outcome::Percent(x)
            | Outcome::Quantity(x, _)
            | Outcome::Duration(x) => x.is_finite(),
            Outcome::Date(_) | Outcome::Time { .. } => true,
        }
    }
}

#[derive(Clone, Copy)]
struct Evaluator<'a> {
    now: &'a Zoned,
    today: Date,
    rule: YearRule,
}

/// Evaluates a parsed query, or `None` for anything undefined: division by zero, mixed
/// dimensions, temperatures added together, a conversion to a different dimension.
pub fn run(parsed: &Parsed, context: &Context) -> Option<Outcome> {
    let today = context.now.date();
    let local = context.now.time_zone();
    let evaluator = Evaluator {
        now: &context.now,
        today,
        rule: YearRule::Next,
    };
    let value = evaluator.eval(&parsed.expr)?;
    let outcome = match (parsed.target.as_ref(), value) {
        (None, Value::Num(x)) => Outcome::Number(x),
        (None, Value::Qty(x, unit)) if unit.dim == Dim::Time && parsed.time_units > 1 => {
            Outcome::Duration(unit.to_base(x))
        }
        (None, Value::Qty(x, unit)) => Outcome::Quantity(x, unit),
        (None, Value::Date(date)) => Outcome::Date(date),
        (None, Value::Clock(time)) => Outcome::Time {
            time,
            place: String::new(),
            weekday: dates::weekday(today),
            offset_minutes: 0,
        },
        (None, Value::At(zoned, place)) => time_at(&zoned, place, local),
        (None | Some(Target::Timespan), Value::Span(seconds)) => Outcome::Duration(seconds),
        (Some(Target::Percent), Value::Num(x)) => Outcome::Percent(x * 100.0),
        (Some(Target::Unit(to)), Value::Qty(x, from)) if to.dim == from.dim => {
            Outcome::Quantity(to.in_unit(from.to_base(x)), to)
        }
        (Some(Target::Timespan), Value::Qty(x, unit)) if unit.dim == Dim::Time => {
            Outcome::Duration(unit.to_base(x))
        }
        (Some(Target::Zone(place)), Value::Clock(time)) => {
            let here = today.to_datetime(time).to_zoned(local.clone()).ok()?;
            time_at(
                &here.with_time_zone(place.tz.clone()),
                place.name.clone(),
                local,
            )
        }
        (Some(Target::Zone(place)), Value::At(zoned, _)) => time_at(
            &zoned.with_time_zone(place.tz.clone()),
            place.name.clone(),
            local,
        ),
        _ => return None,
    };
    if let Outcome::Duration(seconds) = outcome
        && seconds.abs() > 1e15
    {
        return None;
    }
    outcome.is_finite().then_some(outcome)
}

fn time_at(zoned: &Zoned, place: String, local: &TimeZone) -> Outcome {
    let here = zoned.timestamp().to_zoned(local.clone()).offset().seconds();
    Outcome::Time {
        time: zoned.time(),
        place,
        weekday: dates::weekday(zoned.date()),
        offset_minutes: (zoned.offset().seconds() - here) / 60,
    }
}

impl Evaluator<'_> {
    fn eval(&self, expr: &Expr) -> Option<Value> {
        let value = match expr {
            Expr::Num(x) => Value::Num(*x),
            Expr::Qty(x, unit) => Value::Qty(*x, unit),
            Expr::Percent(inner) => Value::Num(number(self.eval(inner)?)? / 100.0),
            Expr::Neg(inner) => match self.eval(inner)? {
                Value::Num(x) => Value::Num(-x),
                Value::Qty(x, unit) => Value::Qty(-x, unit),
                _ => return None,
            },
            Expr::Bin(op @ (BinOp::Add | BinOp::Sub), left, right) => {
                let sign = if *op == BinOp::Add { 1.0 } else { -1.0 };
                let left = self.eval(left)?;
                match right.as_ref() {
                    Expr::Percent(_) => scale(left, 1.0 + sign * number(self.eval(right)?)?)?,
                    _ => combine(left, self.eval(right)?, sign)?,
                }
            }
            Expr::Bin(BinOp::Mul, left, right) => match (self.eval(left)?, self.eval(right)?) {
                (Value::Num(a), Value::Num(b)) => Value::Num(a * b),
                (Value::Qty(a, unit), Value::Num(b)) | (Value::Num(b), Value::Qty(a, unit)) => {
                    scale(Value::Qty(a, unit), b)?
                }
                _ => return None,
            },
            Expr::Bin(BinOp::Div, left, right) => match (self.eval(left)?, self.eval(right)?) {
                (_, Value::Num(0.0)) => return None,
                (Value::Num(a), Value::Num(b)) => Value::Num(a / b),
                (Value::Qty(a, unit), Value::Num(b)) => scale(Value::Qty(a, unit), 1.0 / b)?,
                (Value::Qty(a, u), Value::Qty(b, v))
                    if u.dim == v.dim && u.dim != Dim::Temperature =>
                {
                    let b = v.to_base(b);
                    if b == 0.0 {
                        return None;
                    }
                    Value::Num(u.to_base(a) / b)
                }
                _ => return None,
            },
            Expr::Bin(BinOp::Pow, left, right) => {
                Value::Num(number(self.eval(left)?)?.powf(number(self.eval(right)?)?))
            }
            Expr::Bin(BinOp::Mod, left, right) => {
                let divisor = number(self.eval(right)?)?;
                if divisor == 0.0 {
                    return None;
                }
                Value::Num(number(self.eval(left)?)?.rem_euclid(divisor))
            }
            Expr::Fact(inner) => Value::Num(factorial(number(self.eval(inner)?)?)?),
            Expr::Call(func, arg) => Value::Num(call(*func, self.eval(arg)?)?),
            Expr::PercentOf(percent, base) => {
                scale(self.eval(base)?, number(self.eval(percent)?)?)?
            }
            Expr::Off(percent, base) => {
                scale(self.eval(base)?, 1.0 - number(self.eval(percent)?)?)?
            }
            Expr::Date(date) => Value::Date(*date),
            Expr::MonthDay(month, day) => {
                Value::Date(dates::resolve(*month, *day, self.today, self.rule)?)
            }
            Expr::Today(offset) => {
                Value::Date(self.today.checked_add(i64::from(*offset).days()).ok()?)
            }
            Expr::Clock(time) => Value::Clock(*time),
            Expr::ClockIn(time, place) => Value::At(
                self.today
                    .to_datetime(*time)
                    .to_zoned(place.tz.clone())
                    .ok()?,
                place.name.clone(),
            ),
            Expr::Now => Value::At(self.now.clone(), String::new()),
            Expr::TimeIn(place) => Value::At(
                self.now.with_time_zone(place.tz.clone()),
                place.name.clone(),
            ),
            Expr::Days { to, since, work } => {
                let inner = Evaluator {
                    rule: if *since {
                        YearRule::Last
                    } else {
                        YearRule::Next
                    },
                    ..*self
                };
                let Value::Date(date) = inner.eval(to)? else {
                    return None;
                };
                let (from, until) = if *since {
                    (date, self.today)
                } else {
                    (self.today, date)
                };
                let days = if *work {
                    dates::workdays(from, until)
                } else {
                    i64::from((until - from).get_days())
                };
                Value::Qty(days as f64, units::lookup("d")?)
            }
        };
        let finite = match value {
            Value::Num(x) | Value::Qty(x, _) | Value::Span(x) => x.is_finite(),
            Value::Date(_) | Value::Clock(_) | Value::At(..) => true,
        };
        finite.then_some(value)
    }
}

fn combine(left: Value, right: Value, sign: f64) -> Option<Value> {
    let time = |unit: &Unit| unit.dim == Dim::Time;
    Some(match (left, right) {
        (Value::Date(date), Value::Qty(x, unit)) if time(unit) => {
            Value::Date(dates::shift(date, sign * x, unit.symbol)?)
        }
        (Value::Date(a), Value::Date(b)) if sign < 0.0 => {
            Value::Qty(f64::from((a - b).get_days()), units::lookup("d")?)
        }
        (Value::Clock(clock), Value::Qty(x, unit)) if time(unit) => {
            Value::Clock(dates::shift_time(clock, sign * unit.to_base(x))?)
        }
        (Value::Clock(a), Value::Clock(b)) if sign < 0.0 => {
            Value::Span(dates::seconds_of(a) - dates::seconds_of(b))
        }
        (Value::Span(a), Value::Qty(x, unit)) if time(unit) => {
            Value::Span(a + sign * unit.to_base(x))
        }
        (Value::Span(a), Value::Span(b)) => Value::Span(a + sign * b),
        (left @ (Value::Num(_) | Value::Qty(..)), right @ (Value::Num(_) | Value::Qty(..))) => {
            add(left, right, sign)?
        }
        _ => return None,
    })
}

fn number(value: Value) -> Option<f64> {
    match value {
        Value::Num(x) => Some(x),
        _ => None,
    }
}

fn scale(value: Value, factor: f64) -> Option<Value> {
    match value {
        Value::Num(x) => Some(Value::Num(x * factor)),
        Value::Qty(x, unit) if unit.dim != Dim::Temperature => Some(Value::Qty(x * factor, unit)),
        _ => None,
    }
}

fn add(left: Value, right: Value, sign: f64) -> Option<Value> {
    match (left, right) {
        (Value::Num(a), Value::Num(b)) => Some(Value::Num(a + sign * b)),
        (Value::Qty(a, u), Value::Qty(b, v)) if u.dim == v.dim && u.dim != Dim::Temperature => {
            Some(Value::Qty(a + sign * u.in_unit(v.to_base(b)), u))
        }
        _ => None,
    }
}

fn factorial(n: f64) -> Option<f64> {
    if n < 0.0 || n.fract() != 0.0 || n > 170.0 {
        return None;
    }
    Some((1..=n as u32).map(f64::from).product())
}

fn call(func: Func, value: Value) -> Option<f64> {
    let x = match (func, value) {
        (Func::Sin | Func::Cos | Func::Tan, Value::Qty(x, unit)) if unit.dim == Dim::Angle => {
            unit.to_base(x)
        }
        (_, Value::Num(x)) => x,
        _ => return None,
    };
    let y = match func {
        Func::Sqrt if x < 0.0 => return None,
        Func::Sqrt => x.sqrt(),
        Func::Cbrt => x.cbrt(),
        Func::Sin => x.sin(),
        Func::Cos => x.cos(),
        Func::Tan => x.tan(),
        Func::Asin | Func::Acos if !(-1.0..=1.0).contains(&x) => return None,
        Func::Asin => x.asin(),
        Func::Acos => x.acos(),
        Func::Atan => x.atan(),
        Func::Ln | Func::Log | Func::Log2 if x <= 0.0 => return None,
        Func::Ln => x.ln(),
        Func::Log => x.log10(),
        Func::Log2 => x.log2(),
        Func::Abs => x.abs(),
        Func::Round => x.round(),
        Func::Floor => x.floor(),
        Func::Ceil => x.ceil(),
        Func::Exp => x.exp(),
    };
    let trig = matches!(func, Func::Sin | Func::Cos | Func::Tan);
    Some(if trig && y.abs() < 1e-12 { 0.0 } else { y })
}
