use bip39::Mnemonic;
use rand::Rng as _;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum NumberOfBip39Words {
    Twelve,
    TwentyFour,
}

impl NumberOfBip39Words {
    pub const fn to_word_count(self) -> usize {
        match self {
            NumberOfBip39Words::Twelve => 12,
            NumberOfBip39Words::TwentyFour => 24,
        }
    }

    pub const fn to_entropy_bits(self) -> usize {
        match self {
            NumberOfBip39Words::Twelve => 128,
            NumberOfBip39Words::TwentyFour => 256,
        }
    }

    pub const fn to_entropy_bytes(self) -> usize {
        self.to_entropy_bits() / 8
    }

    pub fn generate_mnemonic(self) -> Mnemonic {
        match self {
            NumberOfBip39Words::Twelve => {
                // 128 / 8  = 16
                let random_bytes = rand::rng().random::<[u8; 16]>();
                Mnemonic::from_entropy(&random_bytes).expect("failed to create mnemonic")
            }
            NumberOfBip39Words::TwentyFour => {
                // 256 / 8  = 32
                let random_bytes = rand::rng().random::<[u8; 32]>();
                Mnemonic::from_entropy(&random_bytes).expect("failed to create mnemonic")
            }
        }
    }

    pub fn in_groups_of(&self, groups_of: usize) -> Vec<Vec<String>> {
        let number_of_groups = self.to_word_count() / groups_of;
        vec![vec![String::new(); groups_of]; number_of_groups]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_of_bip39_words() {
        assert_eq!(NumberOfBip39Words::Twelve.to_entropy_bits(), 128);
        assert_eq!(NumberOfBip39Words::TwentyFour.to_entropy_bits(), 256);

        assert_eq!(NumberOfBip39Words::Twelve.generate_mnemonic().word_count(), 12);

        assert_eq!(NumberOfBip39Words::TwentyFour.generate_mnemonic().word_count(), 24);

        assert_eq!(
            NumberOfBip39Words::Twelve.to_word_count(),
            NumberOfBip39Words::Twelve.generate_mnemonic().word_count()
        );

        assert_eq!(
            NumberOfBip39Words::TwentyFour.to_word_count(),
            NumberOfBip39Words::TwentyFour.generate_mnemonic().word_count()
        );
    }
}
