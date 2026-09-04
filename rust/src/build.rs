const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHORT_HASH: &str = env!("GIT_SHORT_HASH");
const GIT_BRANCH: &str = env!("GIT_BRANCH");
const BUILD_PROFILE: &str = env!("BUILD_PROFILE");

pub fn version() -> String {
    VERSION.to_string()
}

pub fn git_short_hash() -> String {
    GIT_SHORT_HASH.to_string()
}

pub fn git_branch() -> String {
    GIT_BRANCH.to_string()
}

pub fn profile() -> String {
    BUILD_PROFILE.to_string()
}
