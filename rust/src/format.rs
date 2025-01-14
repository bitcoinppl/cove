use numfmt::Numeric;

pub trait NumberFormatter: Numeric {
    fn thousands_int(self) -> String;
    fn thousands_fiat(self) -> String;
}

impl<T: Numeric> NumberFormatter for T {
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

        f.fmt(self).to_string()
    }
}
