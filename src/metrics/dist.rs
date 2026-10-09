//! Distributions: the last N values of a metric and their percentiles.

/// The last `cap` values pushed.
#[derive(Clone, Debug)]
pub struct Ring {
    buf: Vec<f64>,
    next: usize,
    cap: usize,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            next: 0,
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, v: f64) {
        if self.buf.len() < self.cap {
            self.buf.push(v);
        } else {
            self.buf[self.next] = v;
        }
        self.next = (self.next + 1) % self.cap;
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.next = 0;
    }

    /// The latest value.
    pub fn last(&self) -> Option<f64> {
        if self.buf.is_empty() {
            return None;
        }
        Some(self.buf[(self.next + self.cap - 1) % self.cap])
    }

    /// Values in no particular order.
    pub fn values(&self) -> &[f64] {
        &self.buf
    }

    pub fn percentiles(&self) -> Option<Percentiles> {
        Percentiles::of(&self.buf)
    }
}

/// A distribution's summary. Percentiles are nearest-rank: p99 of 1000
/// values is the 990th smallest.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Percentiles {
    pub n: usize,
    pub mean: f64,
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    pub p999: f64,
    pub max: f64,
    /// Mean of the largest 1% (at least one value): for frame times,
    /// 1000 / this is the "1% low" frame rate.
    pub worst_1pct_mean: f64,
    pub total: f64,
}

impl Percentiles {
    pub fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut v = values.to_vec();
        v.sort_by(f64::total_cmp);
        let n = v.len();
        let rank = |q: f64| v[((q * n as f64).ceil() as usize).clamp(1, n) - 1];
        let total: f64 = v.iter().sum();
        let worst = (n / 100).max(1);
        Some(Self {
            n,
            mean: total / n as f64,
            p50: rank(0.5),
            p90: rank(0.9),
            p99: rank(0.99),
            p999: rank(0.999),
            max: v[n - 1],
            worst_1pct_mean: v[n - worst..].iter().sum::<f64>() / worst as f64,
            total,
        })
    }

    /// "1% low" frames per second for frame times in milliseconds: the
    /// rate of the slowest 1% of frames.
    pub fn low_1pct_fps(&self) -> f64 {
        1e3 / self.worst_1pct_mean.max(1e-6)
    }

    /// `p50 1.23 p90 ... max ...` with `f` formatting each value.
    pub fn line(&self, f: impl Fn(f64) -> String) -> String {
        format!(
            "p50 {} p90 {} p99 {} p99.9 {} max {}",
            f(self.p50),
            f(self.p90),
            f(self.p99),
            f(self.p999),
            f(self.max)
        )
    }
}

/// A count in short form: 1234 -> "1.2k", 12345678 -> "12.3M".
pub fn short(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.2}G", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.2}M", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.1}k", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let v: Vec<f64> = (1..=1000).map(f64::from).collect();
        let p = Percentiles::of(&v).unwrap();
        assert_eq!(
            (p.p50, p.p90, p.p99, p.p999, p.max),
            (500.0, 900.0, 990.0, 999.0, 1000.0)
        );
        // The slowest 10 of 1000: 991..=1000.
        assert_eq!(p.worst_1pct_mean, 995.5);
    }

    #[test]
    fn ring_keeps_the_last_values() {
        let mut r = Ring::new(3);
        for v in 1..=5 {
            r.push(v as f64);
        }
        let mut v = r.values().to_vec();
        v.sort_by(f64::total_cmp);
        assert_eq!(v, [3.0, 4.0, 5.0]);
        assert_eq!(r.last(), Some(5.0));
    }
}
