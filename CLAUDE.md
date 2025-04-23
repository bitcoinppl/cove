- ABSOLUTLLY NO SHORTCUTS, NO CHEATING, NO HACKS, NO TRICKS, just the right approach, when moving code around always bring the full code

# Instructions for Extracting Crates

- ALWAYS bring uniffi annotations with the type and add uniffi dep to the crate
- DO NOT create a UDL file, we use only proc macros
- In a new lib.rs if it has uniffi annotations, add the lib.rs add `uniffi::setup_scafolding!` directive
  - DO NOT ADD `uniffi::generate_scaffolding("src/lib.rs").unwrap();`
- Never create a build.rs file, we use only proc macros
- Don't be afraid to add dependencies to the newly extracted crates
- Never add omit_argument_names = true in the uniffi.toml
