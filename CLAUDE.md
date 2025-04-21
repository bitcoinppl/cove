#### IMPORTANT INSTRUCTIONS

- NO short cutes, no cheating, no hacks, no tricks, just the right approach, when moving code around always bring the full code
- When creating a new crate use workspace = true and add the dep to the main Cargo.toml under [workspace.dependencies] if needed
- ALWAYS bring uniffi annotations with the type and add uniffi dep to the crate
- DO NOT create a UDL file, we use only proc macros
- Don't be afraid to add dependencies to the newly extracted crates
- Always omit_argument_names = false in the uniffi.toml
- In a new lib.rs if it has uniffi annotations, add the lib.rs add `uniffi::setup_scafolding!` directive, DO NOT ADD `uniffi::generate_scaffolding("src/lib.rs").unwrap();`
- Never create a build.rs file, we use only proc macros
