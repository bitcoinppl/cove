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
                    0 => format!("{}00", fmt),
                    1 => format!("{}0", fmt),
                    2 => fmt.to_string(),
                    _ => fmt[0..decimal_index + 2].to_string(),
                }
            }

            None => format!("{}.00", fmt),
        }
    }
}

pub fn btc_typing(amount: &str) -> Option<String> {
    let (before_decimal, after_decimal) = split_at_decimal_point(amount);

    let after_decimal_parts = match after_decimal {
        "" => ("", ""),
        after_decimal => (".", after_decimal),
    };

    let int_part_string = before_decimal.parse::<u64>().ok()?.thousands_int();

    let final_string =
        format!("{}{}{}", int_part_string, after_decimal_parts.0, after_decimal_parts.1);

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
}
