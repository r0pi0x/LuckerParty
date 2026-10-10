//! Quantized network forms of what clients only draw (others' bodies:
//! `NetBody`), as Source sends coordinates at 1/32 unit and angles in a
//! fixed number of bits (Valve Developer Wiki, "Networking Entities"):
//! positions to 1/32 Source unit (0.79 mm), velocities to 1/8 unit/s,
//! the look in 16 bits each. The server keeps the values it sends on the
//! grid (`NetBody::quantized`), so what a client decodes is bit for bit
//! what the server has; varints make small numbers (a still body's
//! velocity, an eye offset's zeros) a byte.
//!
//! A client's own predicted state (`OwnState`) is not quantized: it is
//! compared bit for bit with the prediction and replayed from, and the
//! server simulates on from its exact values (`delta` keeps it small).

use std::f32::consts::{FRAC_PI_2, TAU};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::NetBody;

/// One Source unit, m.
const UNIT: f64 = 0.0254;
/// Position step: 1/32 unit, m.
pub const POSITION_STEP: f64 = UNIT / 32.0;
/// Velocity step: 1/8 unit/s, m/s.
pub const VELOCITY_STEP: f64 = UNIT / 8.0;

fn q(v: f32, step: f64) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v as f64 / step).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

fn dq(n: i32, step: f64) -> f32 {
    (n as f64 * step) as f32
}

fn q3(v: [f32; 3], step: f64) -> [i32; 3] {
    v.map(|x| q(x, step))
}

fn dq3(v: [i32; 3], step: f64) -> [f32; 3] {
    v.map(|x| dq(x, step))
}

/// A yaw in [0, 2 pi) as 16 bits.
fn q_yaw(yaw: f32) -> u16 {
    if !yaw.is_finite() {
        return 0;
    }
    ((yaw.rem_euclid(TAU) as f64 / TAU as f64 * 65536.0).round() as u32 & 0xffff) as u16
}

fn dq_yaw(n: u16) -> f32 {
    (n as f64 * TAU as f64 / 65536.0) as f32
}

/// A pitch within straight up and down as 16 bits.
fn q_pitch(pitch: f32) -> i16 {
    if !pitch.is_finite() {
        return 0;
    }
    (pitch.clamp(-FRAC_PI_2, FRAC_PI_2) as f64 / FRAC_PI_2 as f64 * 32767.0).round() as i16
}

fn dq_pitch(n: i16) -> f32 {
    (n.max(-32767) as f64 * FRAC_PI_2 as f64 / 32767.0) as f32
}

/// `NetBody` on the wire.
#[derive(Serialize, Deserialize)]
struct BodyWire {
    origin: [i32; 3],
    velocity: [i32; 3],
    yaw: [u8; 2],
    pitch: [u8; 2],
    eye: [i32; 3],
    flags: u8,
}

impl From<&NetBody> for BodyWire {
    fn from(b: &NetBody) -> Self {
        Self {
            origin: q3(b.origin, POSITION_STEP),
            velocity: q3(b.velocity, VELOCITY_STEP),
            yaw: q_yaw(b.yaw).to_le_bytes(),
            pitch: q_pitch(b.pitch).to_le_bytes(),
            eye: q3(b.eye, POSITION_STEP),
            flags: b.flags,
        }
    }
}

impl From<BodyWire> for NetBody {
    fn from(w: BodyWire) -> Self {
        Self {
            origin: dq3(w.origin, POSITION_STEP),
            velocity: dq3(w.velocity, VELOCITY_STEP),
            yaw: dq_yaw(u16::from_le_bytes(w.yaw)),
            pitch: dq_pitch(i16::from_le_bytes(w.pitch)),
            eye: dq3(w.eye, POSITION_STEP),
            flags: w.flags,
        }
    }
}

impl Serialize for NetBody {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        BodyWire::from(self).serialize(s)
    }
}

impl<'de> Deserialize<'de> for NetBody {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        BodyWire::deserialize(d).map(Self::from)
    }
}

impl NetBody {
    /// The body as a client will decode it: on the network's grid. The
    /// server keeps these values, so both sides hold the same bits.
    pub fn quantized(&self) -> Self {
        BodyWire::from(self).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(origin: [f32; 3], yaw: f32, pitch: f32) -> NetBody {
        NetBody {
            origin,
            velocity: [1.234, -0.5, 6.0],
            yaw,
            pitch,
            eye: [0.0, 1.6256, 0.0],
            flags: 5,
        }
    }

    #[test]
    fn on_the_grid_values_go_through_unchanged() {
        for b in [
            body([0.0, 0.0, 0.0], 0.0, 0.0),
            body([-52.3, 3.25, 98.765_43], 6.283, -1.5707964),
            body([415.9, -100.0, 0.000_3], 3.1, 1.5707964),
            body([1e-9, -0.0, 7.0], -0.2, 0.4),
        ] {
            let q = b.quantized();
            // Within a step of the original.
            for (a, c) in b.origin.iter().zip(q.origin) {
                assert!((a - c).abs() <= (POSITION_STEP as f32) * 0.51, "{a} vs {c}");
            }
            assert!(EyeDiff::of(b.yaw, q.yaw) <= TAU / 65536.0, "{} vs {}", b.yaw, q.yaw);
            assert!((b.pitch - q.pitch).abs() <= FRAC_PI_2 / 32767.0);
            // And a quantized body is its own quantization, also through
            // the wire.
            assert_eq!(q.quantized(), q);
            let bytes = postcard::to_allocvec(&q).unwrap();
            let back: NetBody = postcard::from_bytes(&bytes).unwrap();
            assert_eq!(back, q);
            assert_eq!(back.origin.map(f32::to_bits), q.origin.map(f32::to_bits));
        }
        // A body on a map is ~20 bytes (45 unquantized).
        let b = body([-12.5, 1.25, 30.0], 2.0, 0.1).quantized();
        let n = postcard::to_allocvec(&b).unwrap().len();
        assert!(n <= 24, "{n} bytes");
        // Not a number: zero, not a panic.
        let nan = body([f32::NAN; 3], f32::NAN, f32::INFINITY).quantized();
        assert_eq!(nan.origin, [0.0; 3]);
    }

    /// The short way round between two yaws.
    struct EyeDiff;

    impl EyeDiff {
        fn of(a: f32, b: f32) -> f32 {
            let d = (a - b).rem_euclid(TAU);
            d.min(TAU - d)
        }
    }
}
