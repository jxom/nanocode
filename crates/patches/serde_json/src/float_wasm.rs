use core::fmt::Write;
use core::ops::RangeInclusive;

const TEXT_BYTES: usize = 32;
const NAN: &str = "NaN";
const INFINITY: &str = "inf";
const NEG_INFINITY: &str = "-inf";

mod private {
    pub trait Sealed: Copy + core::fmt::LowerExp {
        const FIXED_DEC_EXP: core::ops::RangeInclusive<i32>;
        fn finite(self) -> bool;
        fn nan(self) -> bool;
        fn negative(self) -> bool;
    }
}

pub trait Float: private::Sealed {}
impl Float for f32 {}
impl Float for f64 {}

impl private::Sealed for f32 {
    const FIXED_DEC_EXP: RangeInclusive<i32> = -6..=12;
    fn finite(self) -> bool {
        self.is_finite()
    }
    fn nan(self) -> bool {
        self.is_nan()
    }
    fn negative(self) -> bool {
        self.is_sign_negative()
    }
}

impl private::Sealed for f64 {
    const FIXED_DEC_EXP: RangeInclusive<i32> = -5..=15;
    fn finite(self) -> bool {
        self.is_finite()
    }
    fn nan(self) -> bool {
        self.is_nan()
    }
    fn negative(self) -> bool {
        self.is_sign_negative()
    }
}

struct Text {
    bytes: [u8; TEXT_BYTES],
    len: usize,
}

impl Text {
    const fn new() -> Self {
        Text {
            bytes: [0; TEXT_BYTES],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        if let Some(slot) = self.bytes.get_mut(self.len) {
            *slot = byte;
            self.len += 1;
        }
    }

    fn extend(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push(byte);
        }
    }

    fn zeros(&mut self, count: usize) {
        for _ in 0..count {
            self.push(b'0');
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(self.bytes.get(..self.len).unwrap_or_default()).unwrap_or_default()
    }
}

impl Write for Text {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        self.extend(text.as_bytes());
        Ok(())
    }
}

pub struct Buffer {
    text: Text,
}

impl Buffer {
    #[inline]
    pub const fn new() -> Self {
        Buffer { text: Text::new() }
    }

    pub fn format<F: Float>(&mut self, f: F) -> &str {
        if f.finite() {
            self.format_finite(f)
        } else if f.nan() {
            NAN
        } else if f.negative() {
            NEG_INFINITY
        } else {
            INFINITY
        }
    }

    pub fn format_finite<F: Float>(&mut self, f: F) -> &str {
        let mut scientific = Text::new();
        let _ = write!(scientific, "{f:e}");
        let written = scientific.bytes.get(..scientific.len).unwrap_or_default();
        let (negative, written) = match written.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, written),
        };
        let marker = written
            .iter()
            .position(|&byte| byte == b'e')
            .unwrap_or(written.len());
        let (mantissa, exponent) = written.split_at(marker);
        let exponent = exponent.get(1..).unwrap_or_default();
        let dec_exp = core::str::from_utf8(exponent)
            .ok()
            .and_then(|exponent| exponent.parse::<i32>().ok())
            .unwrap_or_default();
        let mut digits = Text::new();
        for &byte in mantissa {
            if byte != b'.' {
                digits.push(byte);
            }
        }
        let digits = digits.bytes.get(..digits.len).unwrap_or_default();
        let length = digits.len();

        let text = &mut self.text;
        text.len = 0;
        if negative {
            text.push(b'-');
        }
        if F::FIXED_DEC_EXP.contains(&dec_exp) {
            match usize::try_from(dec_exp + 1) {
                Ok(point) if point > 0 => {
                    if length <= point {
                        text.extend(digits);
                        text.zeros(point - length);
                        text.extend(b".0");
                    } else {
                        let (whole, fraction) = digits.split_at(point);
                        text.extend(whole);
                        text.push(b'.');
                        text.extend(fraction);
                    }
                }
                _ => {
                    text.extend(b"0.");
                    text.zeros(dec_exp.unsigned_abs() as usize - 1);
                    text.extend(digits);
                }
            }
        } else {
            let (first, rest) = digits.split_at(length.min(1));
            text.extend(first);
            if !rest.is_empty() {
                text.push(b'.');
                text.extend(rest);
            }
            let _ = write!(text, "e{dec_exp:+}");
        }
        text.as_str()
    }
}
