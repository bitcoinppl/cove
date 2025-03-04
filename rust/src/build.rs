const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHORT_HASH: &str = env!("GIT_SHORT_HASH");
const BUILD_PROFILE: &str = env!("BUILD_PROFILE");

#[cfg(debug_assertions)]
const IS_RELEASE: bool = false;

#[cfg(not(debug_assertions))]
const IS_RELEASE: bool = true;

pub fn version() -> String {
    VERSION.to_string()
}

pub fn git_short_hash() -> String {
    GIT_SHORT_HASH.to_string()
}

pub fn is_release() -> bool {
    IS_RELEASE
}

pub fn profile() -> String {
    BUILD_PROFILE.to_string()
}
