//! Values carried by outputs into inputs, and their conversions
//! (specs/source/entity_io.md, "Delivering an input; parameter typing").

use bevy::math::Vec3;

use super::Who;

/// A value an output carries (or a parameter override, always a string).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    /// No value (OnTrigger, OnStartTouch...).
    #[default]
    Void,
    Str(String),
    Int(i32),
    Float(f32),
    Bool(bool),
    Vector(Vec3),
    /// An entity (its name converts to a string).
    Ent(Option<Who>),
}

/// C `atoi`: optional whitespace and sign, then digits; anything else 0.
pub fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for c in rest.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        v = (v * 10 + (c - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// C `atof`: the longest leading float ("2.5abc" -> 2.5, "" -> 0).
pub fn atof(s: &str) -> f32 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut end = 0;
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut digits = false;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits = true;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits = true;
        }
    }
    if digits {
        end = i;
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            let start = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                end = j;
            }
        }
    }
    s[..end].parse().unwrap_or(0.0)
}

/// C `printf("%g")` of a float: 6 significant digits, trailing zeros
/// dropped, exponent form below 1e-4 or from 1e6 ("1e+06").
pub fn fmt_g(f: f32) -> String {
    let f = f as f64;
    if f == 0.0 {
        return "0".into();
    }
    if !f.is_finite() {
        return if f.is_nan() { "nan".into() } else if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // Round to 6 significant digits first; the exponent decides the form.
    let sci = format!("{:.5e}", f);
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let trim = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if exp < -4 || exp >= 6 {
        let m = trim(mantissa.to_string());
        format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let decimals = (5 - exp).max(0) as usize;
        trim(format!("{:.*}", decimals, f))
    }
}

impl Value {
    /// Integer input: strings by atoi, floats truncated; no value and
    /// vectors are rejected.
    pub fn to_int(&self) -> Option<i32> {
        match self {
            Value::Str(s) => Some(atoi(s)),
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i32),
            Value::Bool(b) => Some(*b as i32),
            _ => None,
        }
    }

    pub fn to_float(&self) -> Option<f32> {
        match self {
            Value::Str(s) => Some(atof(s)),
            Value::Int(i) => Some(*i as f32),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(*b as i32 as f32),
            _ => None,
        }
    }

    /// Boolean input: strings by atoi != 0 (so "true" is false).
    pub fn to_bool(&self) -> Option<bool> {
        match self {
            Value::Str(s) => Some(atoi(s) != 0),
            Value::Int(i) => Some(*i != 0),
            Value::Float(f) => Some(*f != 0.0),
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn to_vector(&self) -> Option<Vec3> {
        match self {
            Value::Str(s) => Some(crate::map::entities::parse_vector(s)),
            Value::Vector(v) => Some(*v),
            _ => None,
        }
    }

    /// String input: strings as is, no value as "", entities by name
    /// (`name_of`); numbers are rejected (the original refuses them).
    pub fn to_str(&self, name_of: impl Fn(Who) -> String) -> Option<String> {
        match self {
            Value::Void => Some(String::new()),
            Value::Str(s) => Some(s.clone()),
            Value::Ent(e) => Some(e.map(name_of).unwrap_or_default()),
            _ => None,
        }
    }

    /// The value as text, for "any" inputs (logic_case InValue).
    pub fn as_text(&self, name_of: impl Fn(Who) -> String) -> String {
        match self {
            Value::Void => String::new(),
            Value::Str(s) => s.clone(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => fmt_g(*f),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Vector(v) => format!("[{} {} {}]", fmt_g(v.x), fmt_g(v.y), fmt_g(v.z)),
            Value::Ent(e) => e.map(name_of).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_conversions() {
        assert_eq!(atoi("2.7"), 2);
        assert_eq!(atoi("  -12x"), -12);
        assert_eq!(atoi("true"), 0);
        assert_eq!(atof("2.5abc"), 2.5);
        assert_eq!(atof(""), 0.0);
        assert_eq!(atof("1e3"), 1000.0);
        assert_eq!(fmt_g(3.0), "3");
        assert_eq!(fmt_g(2.5), "2.5");
        assert_eq!(fmt_g(1_000_000.0), "1e+06");
        assert_eq!(fmt_g(0.0001), "0.0001");
        assert_eq!(fmt_g(-0.5), "-0.5");
        assert_eq!(fmt_g(123456.0), "123456");
    }

    #[test]
    fn typing_rules() {
        assert_eq!(Value::Str("true".into()).to_bool(), Some(false));
        assert_eq!(Value::Str("1".into()).to_bool(), Some(true));
        assert_eq!(Value::Void.to_float(), None);
        assert_eq!(Value::Float(3.0).to_str(|_| String::new()), None);
        assert_eq!(Value::Str("1.9".into()).to_int(), Some(1));
        assert_eq!(Value::Str("1.9".into()).to_float(), Some(1.9));
        assert_eq!(Value::Float(-2.7).to_int(), Some(-2));
    }
}
