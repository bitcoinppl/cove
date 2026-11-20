//! `cargo xtask check-error-text`: fails when an `#[error("...")]` message formats a value with
//! Debug (`{:?}`). The apps show an error's Display text in alerts, so a Debug placeholder there
//! reaches the user as a dump such as `NodeConnectionFailed("...")`

use std::{fs, path::Path};

use clap::Args;
use color_eyre::{
    eyre::{bail, Context},
    Result,
};
use syn::{spanned::Spanned, visit::Visit, Attribute, LitStr};

use crate::common::print_success;

#[derive(Debug, Args)]
pub struct CheckErrorText {}

pub fn run(_args: CheckErrorText) -> Result<()> {
    let rust_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is in rust/xtask");
    let mut failures = Vec::new();
    let mut dirs = vec![rust_dir.join("src"), rust_dir.join("crates")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }

            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }

            let source = fs::read_to_string(&path)?;
            let file =
                syn::parse_file(&source).with_context(|| format!("parsing {}", path.display()))?;
            let mut visitor = ErrorAttributes(Vec::new());
            visitor.visit_file(&file);

            let relative = path.strip_prefix(rust_dir).unwrap_or(&path).display().to_string();
            failures.extend(visitor.0.iter().map(|line| format!("rust/{relative}:{line}")));
        }
    }

    if !failures.is_empty() {
        failures.sort();
        eprintln!("{}", failures.join("\n"));
        bail!(
            "{} #[error] message(s) format a value with Debug ({{:?}}); use Display",
            failures.len()
        );
    }

    print_success("No Debug formatting in #[error] messages");
    Ok(())
}

/// The lines of the `#[error("...")]` attributes whose message has a Debug placeholder
struct ErrorAttributes(Vec<usize>);

impl Visit<'_> for ErrorAttributes {
    fn visit_attribute(&mut self, attribute: &Attribute) {
        let message = attribute.path().is_ident("error").then(|| {
            attribute.parse_args_with(|input: syn::parse::ParseStream| {
                let message: LitStr = input.parse()?;
                input.parse::<proc_macro2::TokenStream>()?;
                Ok(message.value())
            })
        });

        if message.and_then(syn::Result::ok).is_some_and(|message| has_debug_placeholder(&message))
        {
            self.0.push(attribute.span().start().line);
        }
    }
}

/// Whether a format string has a placeholder such as `{:?}`, `{0:#?}` or `{name:x?}`
fn has_debug_placeholder(format: &str) -> bool {
    // `{{` is a literal brace, so dropping it leaves only real placeholders
    let format = format.replace("{{", "");
    format
        .split('{')
        .skip(1)
        .any(|rest| rest.split('}').next().is_some_and(|spec| spec.trim_end().ends_with('?')))
}

#[cfg(test)]
mod tests {
    use syn::visit::Visit;

    use super::{has_debug_placeholder, ErrorAttributes};

    #[test]
    fn finds_debug_in_error_messages() {
        let source = r#"
enum Error {
    #[error("unsupported descriptor type: {0:?}")]
    Unsupported(DescriptorType),
    #[error("node error: {0}")]
    Node(String),
    #[error(transparent)]
    Other(#[from] OtherError),
    #[error("expected {expected:?}, got {}", name(.0))]
    Mismatch { expected: Kind },
}
"#;
        let mut visitor = ErrorAttributes(Vec::new());
        visitor.visit_file(&syn::parse_file(source).unwrap());
        assert_eq!(visitor.0, vec![3, 9]);
    }

    #[test]
    fn debug_placeholders() {
        assert!(has_debug_placeholder("{:?}"));
        assert!(has_debug_placeholder("a {0:#?} b"));
        assert!(has_debug_placeholder("{value:x?}"));
        assert!(!has_debug_placeholder("{value} {0:>8} {{x:?}}"));
        assert!(!has_debug_placeholder("why?"));
    }
}
