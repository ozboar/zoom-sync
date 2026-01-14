use std::fmt::Debug;
use std::ops::Not;

use crate::abi::Arg;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DumbFloat16(u16);

impl DumbFloat16 {
    /// 0.0
    pub const MIN: DumbFloat16 = DumbFloat16(u16::MIN);
    /// 655.33997
    pub const MAX: DumbFloat16 = DumbFloat16(u16::MAX);
    pub const MIN_F32: f32 = 0.0;
    pub const MAX_F32: f32 = 655.33997;

    /// Create a new float, clamping at the minimum and maximum values
    pub fn new(mut float: f32) -> Self {
        if float <= Self::MIN_F32 {
            return Self::MIN;
        }
        if float >= Self::MAX_F32 {
            return Self::MAX;
        }

        let mut n = 0u16;
        let mut current = 327.68f32 * 2.0;
        for _ in 0..16 {
            n <<= 1;
            current /= 2.0;
            if float >= current - 0.005 {
                float -= current;
                n |= 1;
            }
        }
        Self(n)
    }

    /// Convert a floating point number to the byte representation.
    #[inline(always)]
    pub fn to_bit_repr(&self) -> [u8; 2] {
        self.0.to_be_bytes()
    }

    /// Convert a byte representation back to a floating point.
    #[inline(always)]
    pub fn from_bit_repr(repr: [u8; 2]) -> Self {
        Self(u16::from_be_bytes(repr))
    }
}

impl From<&DumbFloat16> for f32 {
    fn from(value: &DumbFloat16) -> f32 {
        let mut n = value.0;
        let mut current = 0.01;
        let mut result = 0.00;
        while n != 0 {
            if n & 1 == 1 {
                result += current;
            }
            current *= 2.0;
            n >>= 1;
        }
        result
    }
}

impl TryFrom<f32> for DumbFloat16 {
    type Error = ();
    #[inline(always)]
    fn try_from(value: f32) -> Result<Self, Self::Error> {
        (Self::MIN_F32..=Self::MAX_F32)
            .contains(&value)
            .not()
            .then_some(DumbFloat16::new(value))
            .ok_or(())
    }
}

impl Debug for DumbFloat16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BadFloat16").field(&f32::from(self)).finish()
    }
}

impl Arg for DumbFloat16 {
    const SIZE: usize = 2;
    #[inline(always)]
    fn to_bytes(&self) -> Vec<u8> {
        self.to_bit_repr().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for i in 0..u16::MAX {
            let repr = i.to_be_bytes();
            let x = DumbFloat16::from_bit_repr(repr);
            let y = x.to_bit_repr();
            assert_eq!(repr, y, "{x:?}");
            println!("{x:?}");
        }
    }
}
