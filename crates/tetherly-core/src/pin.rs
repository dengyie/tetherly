// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CoreError;
use crate::ports::Rng;

pub fn generate_pin(rng: &dyn Rng) -> Result<[u8; 8], CoreError> {
    let mut buf = [0u8; 8];
    rng.fill_bytes(&mut buf)?;
    let mut digits = [0u8; 8];
    for (i, b) in buf.iter().enumerate() {
        digits[i] = b % 10;
    }
    Ok(digits)
}

pub fn format_pin(digits: &[u8; 8]) -> String {
    format!(
        "{}{}{}{} {}{}{}{}",
        digits[0], digits[1], digits[2], digits[3], digits[4], digits[5], digits[6], digits[7]
    )
}

pub fn parse_pin(input: &str) -> Result<[u8; 8], CoreError> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() != 8 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CoreError::InvalidPin);
    }
    let mut out = [0u8; 8];
    for (i, b) in compact.bytes().enumerate() {
        out[i] = b - b'0';
    }
    Ok(out)
}

pub fn pin_ascii(digits: &[u8; 8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, d) in digits.iter().enumerate() {
        out[i] = b'0' + *d;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Rng;

    struct Fixed;

    impl Rng for Fixed {
        fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError> {
            dest.copy_from_slice(&[2, 5, 1, 7, 0, 3, 9, 4][..dest.len()]);
            Ok(())
        }
    }

    struct FailingRng;

    impl Rng for FailingRng {
        fn fill_bytes(&self, _dest: &mut [u8]) -> Result<(), CoreError> {
            Err(CoreError::Rng)
        }
    }

    #[test]
    fn grouped_display_and_parse() {
        let pin = generate_pin(&Fixed).unwrap();
        assert_eq!(format_pin(&pin), "2517 0394");
        assert_eq!(parse_pin("2517 0394").unwrap(), pin);
        assert_eq!(parse_pin("25170394").unwrap(), pin);
        assert!(parse_pin("123").is_err());
        assert_eq!(&pin_ascii(&pin), b"25170394");
    }

    #[test]
    fn generate_pin_fails_closed_on_rng_error() {
        assert_eq!(generate_pin(&FailingRng).unwrap_err(), CoreError::Rng);
    }
}
