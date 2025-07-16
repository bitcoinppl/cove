use numfmt::Numeric;

use crate::split_at_decimal_point;

pub trait NumberFormatter: Numeric {
    fn thousands_int(self) -> String;
    fn thousands_fiat(self) -> String;
    fn thousands(self) -> String;
}

impl<T: Numeric> NumberFormatter for T {
    fn thousands(self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }

        let mut f = numfmt::Formatter::new().separator(',').unwrap();

        f.fmt(self).to_string()
    }

    fn thousands_int(self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }

        let mut f = numfmt::Formatter::new()
            .separator(',')
            .unwrap()
            .precision(numfmt::Precision::Decimals(0));

        f.fmt(self).to_string()
    }

    fn thousands_fiat(self) -> String {
        if self.is_zero() {
            return "0.00".to_string();
        }

        let mut f = numfmt::Formatter::new()
            .separator(',')
            .unwrap()
            .precision(numfmt::Precision::Decimals(2));

        let fmt = f.fmt(self);

        // HACK: actually make sure we always have 2 decimals
        let last_index = fmt.len() - 1;
        match memchr::memchr(b'.', fmt.as_bytes()) {
            Some(decimal_index) => {
                let decimals = last_index - decimal_index;
                match decimals {
                    0 => format!("{fmt}00"),
                    1 => format!("{fmt}0"),
                    2 => fmt.to_string(),
                    _ => fmt[0..decimal_index + 2].to_string(),
                }
            }

            None => format!("{fmt}.00"),
        }
    }
}

pub fn btc_typing(amount: &str) -> Option<String> {
    if amount == "." {
        return Some("0.".to_string());
    }

    let (before_decimal, decimal, after_decimal) = split_at_decimal_point(amount);

    let int_part_string = match before_decimal {
        "" => "0".to_string(),
        before => before.parse::<u64>().ok()?.thousands_int(),
    };

    let max_decimal_places_to_take = after_decimal.len().min(8);
    let decimal_places = &after_decimal[..max_decimal_places_to_take];

    let final_string = format!("{int_part_string}{decimal}{decimal_places}");

    Some(final_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_formatter() {
        let number = 20_000;
        let formatted = number.thousands_fiat();
        assert_eq!(formatted, "20,000.00");
    }

    #[test]
    fn test_btc_typing() {
        assert_eq!(btc_typing("0.00"), Some("0.00".to_string()));
        assert_eq!(btc_typing("0."), Some("0.".to_string()));
        assert_eq!(btc_typing("12345.123456789100"), Some("12,345.12345678".to_string()));
        assert_eq!(btc_typing(".00"), Some("0.00".to_string()));
        assert_eq!(btc_typing("."), Some("0.".to_string()));
    }
}
