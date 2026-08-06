//! Fixed-point money arithmetic.
//!
//! Every monetary value in this crate is an `i64` count of atomic units. There is
//! exactly one way to combine them — [`mul_div`] — and it widens to `i128` for the
//! intermediate product so `price * qty` cannot silently wrap.
//!
//! Floats are not used anywhere in this crate and must not be introduced.

use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    #[error("arithmetic overflow")]
    Overflow,
    #[error("division by zero")]
    DivideByZero,
}

/// Which way to break a tie. Defined against the number line, not against zero,
/// so the meaning does not flip for negative values (it will once perps land).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Toward negative infinity.
    Down,
    /// Toward positive infinity.
    Up,
}

/// `a * b / denom`, computed in `i128` and rounded explicitly.
pub fn mul_div(a: i64, b: i64, denom: i64, rounding: Rounding) -> Result<i64, MathError> {
    if denom == 0 {
        return Err(MathError::DivideByZero);
    }

    let n = (a as i128) * (b as i128);
    let d = denom as i128;

    let trunc = n / d;
    let rem = n % d;

    // `rem` carries the sign of `n`, so comparing its sign to `d`'s tells us which
    // side of the true quotient `trunc` landed on.
    let adjusted = if rem == 0 {
        trunc
    } else {
        match rounding {
            Rounding::Down if (rem < 0) != (d < 0) => trunc - 1,
            Rounding::Up if (rem < 0) == (d < 0) => trunc + 1,
            _ => trunc,
        }
    };

    i64::try_from(adjusted).map_err(|_| MathError::Overflow)
}

/// `10^exp` as an `i64`. Errors rather than wrapping past `10^18`.
pub fn pow10(exp: u32) -> Result<i64, MathError> {
    10i64.checked_pow(exp).ok_or(MathError::Overflow)
}

pub fn checked_add(a: i64, b: i64) -> Result<i64, MathError> {
    a.checked_add(b).ok_or(MathError::Overflow)
}

pub fn checked_sub(a: i64, b: i64) -> Result<i64, MathError> {
    a.checked_sub(b).ok_or(MathError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_division_ignores_rounding_mode() {
        assert_eq!(mul_div(100, 3, 10, Rounding::Down).unwrap(), 30);
        assert_eq!(mul_div(100, 3, 10, Rounding::Up).unwrap(), 30);
    }

    #[test]
    fn positive_values_round_as_named() {
        assert_eq!(mul_div(10, 1, 3, Rounding::Down).unwrap(), 3);
        assert_eq!(mul_div(10, 1, 3, Rounding::Up).unwrap(), 4);
    }

    #[test]
    fn negative_values_round_toward_the_named_infinity() {
        // -10/3 is -3.33; Down (floor) is -4, Up (ceil) is -3.
        assert_eq!(mul_div(-10, 1, 3, Rounding::Down).unwrap(), -4);
        assert_eq!(mul_div(-10, 1, 3, Rounding::Up).unwrap(), -3);
        // A negative denominator must give the same answers.
        assert_eq!(mul_div(10, 1, -3, Rounding::Down).unwrap(), -4);
        assert_eq!(mul_div(10, 1, -3, Rounding::Up).unwrap(), -3);
    }

    #[test]
    fn product_that_overflows_i64_still_computes() {
        // 1e18 * 1e8 overflows i64 as a product but the quotient fits.
        let got = mul_div(
            1_000_000_000_000_000_000,
            100_000_000,
            100_000_000,
            Rounding::Down,
        );
        assert_eq!(got.unwrap(), 1_000_000_000_000_000_000);
    }

    #[test]
    fn quotient_that_cannot_fit_is_an_error() {
        let got = mul_div(i64::MAX, 4, 1, Rounding::Down);
        assert_eq!(got, Err(MathError::Overflow));
    }

    #[test]
    fn divide_by_zero_is_an_error() {
        assert_eq!(
            mul_div(1, 1, 0, Rounding::Down),
            Err(MathError::DivideByZero)
        );
    }

    #[test]
    fn pow10_rejects_beyond_i64() {
        assert_eq!(pow10(8).unwrap(), 100_000_000);
        assert_eq!(pow10(18).unwrap(), 1_000_000_000_000_000_000);
        assert_eq!(pow10(19), Err(MathError::Overflow));
    }
}
