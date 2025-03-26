use std::hash::{DefaultHasher, Hash, Hasher as _};

pub fn calculate_hash<T>(t: &T) -> u64
where
    T: Hash + ?Sized,
{
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

pub fn generate_random_chain_code() -> [u8; 32] {
    use rand::Rng as _;

    let rng = &mut rand::rng();
    let mut chain_code = [0u8; 32];
    rng.fill(&mut chain_code);

    chain_code
}

mod ffi {
    #[uniffi::export]
    fn hex_encode(bytes: Vec<u8>) -> String {
        hex::encode(bytes)
    }

    #[uniffi::export]
    pub fn generate_random_chain_code() -> String {
        use rand::Rng as _;

        let rng = &mut rand::rng();
        let mut chain_code = [0u8; 32];
        rng.fill(&mut chain_code);

        hex::encode(chain_code)
    }
}
