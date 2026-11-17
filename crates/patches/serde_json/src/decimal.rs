const CAPACITY: usize = 800;
const MAX_SHIFT: u32 = 60;
const POWERS: [u32; 9] = [1, 3, 6, 9, 13, 16, 19, 23, 26];

struct Format {
    bias: i32,
    exponent_bits: u32,
    mantissa_bits: u32,
}

const F32: Format = Format {
    bias: -127,
    exponent_bits: 8,
    mantissa_bits: 23,
};

const F64: Format = Format {
    bias: -1023,
    exponent_bits: 11,
    mantissa_bits: 52,
};

struct Decimal {
    digits: [u8; CAPACITY],
    count: usize,
    point: i32,
    truncated: bool,
}

pub(crate) fn parse(text: &[u8], single: bool) -> Option<f64> {
    let mut decimal = Decimal::read(text)?;
    if single {
        let bits = decimal.bits(&F32)?;
        Some(f64::from(f32::from_bits(u32::try_from(bits).ok()?)))
    } else {
        Some(f64::from_bits(decimal.bits(&F64)?))
    }
}

impl Decimal {
    fn read(text: &[u8]) -> Option<Self> {
        let mut decimal = Self {
            digits: [0; CAPACITY],
            count: 0,
            point: 0,
            truncated: false,
        };
        let mut seen = 0i32;
        let mut dot = false;
        let mut any = false;
        let mut index = 0;
        while let Some(&byte) = text.get(index) {
            match byte {
                b'.' if !dot => {
                    dot = true;
                    decimal.point = seen;
                }
                b'0'..=b'9' => {
                    any = true;
                    if byte == b'0' && seen == 0 {
                        decimal.point -= 1;
                    } else {
                        seen = seen.saturating_add(1);
                        if let Some(slot) = decimal.digits.get_mut(decimal.count) {
                            *slot = byte - b'0';
                            decimal.count += 1;
                        } else if byte != b'0' {
                            decimal.truncated = true;
                        }
                    }
                }
                _ => break,
            }
            index += 1;
        }
        if !any {
            return None;
        }
        if !dot {
            decimal.point = seen;
        }
        if let Some(b'e' | b'E') = text.get(index) {
            index += 1;
            let negative = match text.get(index) {
                Some(b'-') => {
                    index += 1;
                    true
                }
                Some(b'+') => {
                    index += 1;
                    false
                }
                _ => false,
            };
            let start = index;
            let mut exponent = 0i32;
            while let Some(&byte @ b'0'..=b'9') = text.get(index) {
                if exponent < 10000 {
                    exponent = exponent * 10 + i32::from(byte - b'0');
                }
                index += 1;
            }
            if index == start {
                return None;
            }
            decimal.point += if negative { -exponent } else { exponent };
        }
        (index == text.len()).then_some(decimal)
    }

    fn bits(&mut self, format: &Format) -> Option<u64> {
        let mut exponent;
        let mut mantissa;
        let overflow = (1i32 << format.exponent_bits) - 1;
        if self.count == 0 || self.point < -330 {
            mantissa = 0;
            exponent = format.bias;
        } else if self.point > 310 {
            return None;
        } else {
            exponent = 0;
            while self.point > 0 {
                let shift = usize::try_from(self.point)
                    .ok()
                    .and_then(|point| POWERS.get(point))
                    .map_or(27, |&shift| shift as i32);
                self.shift(-shift);
                exponent += shift;
            }
            while self.point < 0 || (self.point == 0 && self.digits.first().is_some_and(|&digit| digit < 5)) {
                let shift = usize::try_from(-self.point)
                    .ok()
                    .and_then(|point| POWERS.get(point))
                    .map_or(27, |&shift| shift as i32);
                self.shift(shift);
                exponent -= shift;
            }
            exponent -= 1;
            if exponent < format.bias + 1 {
                let shift = format.bias + 1 - exponent;
                self.shift(-shift);
                exponent += shift;
            }
            if exponent - format.bias >= overflow {
                return None;
            }
            self.shift(1 + format.mantissa_bits as i32);
            mantissa = self.rounded();
            if mantissa == 2 << format.mantissa_bits {
                mantissa >>= 1;
                exponent += 1;
                if exponent - format.bias >= overflow {
                    return None;
                }
            }
            if mantissa & (1 << format.mantissa_bits) == 0 {
                exponent = format.bias;
            }
        }
        let biased = u64::try_from((exponent - format.bias) & overflow).ok()?;
        Some((mantissa & ((1 << format.mantissa_bits) - 1)) | (biased << format.mantissa_bits))
    }

    fn shift(&mut self, mut shift: i32) {
        if self.count == 0 {
            return;
        }
        while shift > MAX_SHIFT as i32 {
            self.left(MAX_SHIFT);
            shift -= MAX_SHIFT as i32;
        }
        while shift < -(MAX_SHIFT as i32) {
            self.right(MAX_SHIFT);
            shift += MAX_SHIFT as i32;
        }
        match shift {
            1.. => self.left(shift.unsigned_abs()),
            ..0 => self.right(shift.unsigned_abs()),
            0 => {}
        }
    }

    fn left(&mut self, shift: u32) {
        let mut reversed = [0u8; CAPACITY + 20];
        let mut written = 0;
        let mut carry = 0u64;
        for &digit in self.digits.get(..self.count).unwrap_or_default().iter().rev() {
            carry += u64::from(digit) << shift;
            if let Some(slot) = reversed.get_mut(written) {
                *slot = (carry % 10) as u8;
            }
            carry /= 10;
            written += 1;
        }
        while carry > 0 {
            if let Some(slot) = reversed.get_mut(written) {
                *slot = (carry % 10) as u8;
            }
            carry /= 10;
            written += 1;
        }
        self.point += (written - self.count) as i32;
        self.count = 0;
        for &digit in reversed.get(..written).unwrap_or_default().iter().rev() {
            if let Some(slot) = self.digits.get_mut(self.count) {
                *slot = digit;
                self.count += 1;
            } else if digit != 0 {
                self.truncated = true;
            }
        }
        self.trim();
    }

    fn right(&mut self, shift: u32) {
        let mut read = 0;
        let mut written = 0;
        let mut value = 0u64;
        while value >> shift == 0 {
            if let Some(&digit) = self.digits.get(read).filter(|_| read < self.count) {
                value = value * 10 + u64::from(digit);
            } else if value == 0 {
                self.count = 0;
                return;
            } else {
                value *= 10;
            }
            read += 1;
        }
        self.point -= read as i32 - 1;
        let mask = (1u64 << shift) - 1;
        while read < self.count {
            let digit = self.digits.get(read).copied().unwrap_or_default();
            if let Some(slot) = self.digits.get_mut(written) {
                *slot = (value >> shift) as u8;
            }
            written += 1;
            value = (value & mask) * 10 + u64::from(digit);
            read += 1;
        }
        while value > 0 {
            let digit = (value >> shift) as u8;
            value &= mask;
            if let Some(slot) = self.digits.get_mut(written) {
                *slot = digit;
                written += 1;
            } else if digit > 0 {
                self.truncated = true;
            }
            value *= 10;
        }
        self.count = written;
        self.trim();
    }

    fn trim(&mut self) {
        while self.count > 0 && self.digits.get(self.count - 1) == Some(&0) {
            self.count -= 1;
        }
        if self.count == 0 {
            self.point = 0;
        }
    }

    fn rounded(&self) -> u64 {
        if self.point > 20 {
            return u64::MAX;
        }
        let Ok(point) = usize::try_from(self.point) else {
            return 0;
        };
        let mut value = 0u64;
        for index in 0..point {
            value = value * 10 + u64::from(self.digits.get(index).filter(|_| index < self.count).copied().unwrap_or_default());
        }
        let up = point < self.count
            && match self.digits.get(point) {
                Some(5) if point + 1 == self.count => {
                    self.truncated || point.checked_sub(1).and_then(|index| self.digits.get(index)).is_some_and(|digit| digit % 2 == 1)
                }
                Some(&digit) => digit >= 5,
                None => false,
            };
        value + u64::from(up)
    }
}
