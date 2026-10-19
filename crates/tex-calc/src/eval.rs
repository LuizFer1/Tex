use crate::parser::{BinOp, Expr, Func, Parsed, Target};
use crate::units::{Dim, Unit};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Num(f64),
    Qty(f64, &'static Unit),
}

/// What a query evaluates to, ready to format. `Duration` is in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Number(f64),
    Percent(f64),
    Quantity(f64, &'static Unit),
    Duration(f64),
}

impl Outcome {
    fn is_finite(self) -> bool {
        match self {
            Outcome::Number(x)
            | Outcome::Percent(x)
            | Outcome::Quantity(x, _)
            | Outcome::Duration(x) => x.is_finite(),
        }
    }
}

/// Evaluates a parsed query, or `None` for anything undefined: division by zero, mixed
/// dimensions, temperatures added together, a conversion to a different dimension.
pub fn run(parsed: &Parsed) -> Option<Outcome> {
    let value = eval(&parsed.expr)?;
    let outcome = match (parsed.target, value) {
        (None, Value::Num(x)) => Outcome::Number(x),
        (None, Value::Qty(x, unit)) if unit.dim == Dim::Time && parsed.time_units > 1 => {
            Outcome::Duration(unit.to_base(x))
        }
        (None, Value::Qty(x, unit)) => Outcome::Quantity(x, unit),
        (Some(Target::Percent), Value::Num(x)) => Outcome::Percent(x * 100.0),
        (Some(Target::Unit(to)), Value::Qty(x, from)) if to.dim == from.dim => {
            Outcome::Quantity(to.in_unit(from.to_base(x)), to)
        }
        (Some(Target::Timespan), Value::Qty(x, unit)) if unit.dim == Dim::Time => {
            Outcome::Duration(unit.to_base(x))
        }
        _ => return None,
    };
    outcome.is_finite().then_some(outcome)
}

fn eval(expr: &Expr) -> Option<Value> {
    let value = match expr {
        Expr::Num(x) => Value::Num(*x),
        Expr::Qty(x, unit) => Value::Qty(*x, unit),
        Expr::Percent(inner) => Value::Num(number(eval(inner)?)? / 100.0),
        Expr::Neg(inner) => match eval(inner)? {
            Value::Num(x) => Value::Num(-x),
            Value::Qty(x, unit) => Value::Qty(-x, unit),
        },
        Expr::Bin(op @ (BinOp::Add | BinOp::Sub), left, right) => {
            let sign = if *op == BinOp::Add { 1.0 } else { -1.0 };
            let left = eval(left)?;
            match right.as_ref() {
                Expr::Percent(_) => scale(left, 1.0 + sign * number(eval(right)?)?)?,
                _ => add(left, eval(right)?, sign)?,
            }
        }
        Expr::Bin(BinOp::Mul, left, right) => match (eval(left)?, eval(right)?) {
            (Value::Num(a), Value::Num(b)) => Value::Num(a * b),
            (Value::Qty(a, unit), Value::Num(b)) | (Value::Num(b), Value::Qty(a, unit)) => {
                scale(Value::Qty(a, unit), b)?
            }
            _ => return None,
        },
        Expr::Bin(BinOp::Div, left, right) => match (eval(left)?, eval(right)?) {
            (_, Value::Num(0.0)) => return None,
            (Value::Num(a), Value::Num(b)) => Value::Num(a / b),
            (Value::Qty(a, unit), Value::Num(b)) => scale(Value::Qty(a, unit), 1.0 / b)?,
            (Value::Qty(a, u), Value::Qty(b, v)) if u.dim == v.dim && u.dim != Dim::Temperature => {
                let b = v.to_base(b);
                if b == 0.0 {
                    return None;
                }
                Value::Num(u.to_base(a) / b)
            }
            _ => return None,
        },
        Expr::Bin(BinOp::Pow, left, right) => {
            Value::Num(number(eval(left)?)?.powf(number(eval(right)?)?))
        }
        Expr::Bin(BinOp::Mod, left, right) => {
            let divisor = number(eval(right)?)?;
            if divisor == 0.0 {
                return None;
            }
            Value::Num(number(eval(left)?)?.rem_euclid(divisor))
        }
        Expr::Fact(inner) => Value::Num(factorial(number(eval(inner)?)?)?),
        Expr::Call(func, arg) => Value::Num(call(*func, eval(arg)?)?),
        Expr::PercentOf(percent, base) => scale(eval(base)?, number(eval(percent)?)?)?,
        Expr::Off(percent, base) => scale(eval(base)?, 1.0 - number(eval(percent)?)?)?,
    };
    let finite = match value {
        Value::Num(x) | Value::Qty(x, _) => x.is_finite(),
    };
    finite.then_some(value)
}

fn number(value: Value) -> Option<f64> {
    match value {
        Value::Num(x) => Some(x),
        Value::Qty(..) => None,
    }
}

fn scale(value: Value, factor: f64) -> Option<Value> {
    match value {
        Value::Num(x) => Some(Value::Num(x * factor)),
        Value::Qty(x, unit) if unit.dim != Dim::Temperature => Some(Value::Qty(x * factor, unit)),
        Value::Qty(..) => None,
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
