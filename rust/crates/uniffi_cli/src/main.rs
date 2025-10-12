use std::env;

fn main() {
    // check if any argument contains "kotlin"
    let args: Vec<String> = env::args().collect();
    let is_kotlin = args.iter().any(|arg| arg.to_lowercase().contains("kotlin"));

    if is_kotlin {
        uniffi::uniffi_bindgen_main();
    } else {
        uniffi::uniffi_bindgen_swift();
    }
}
