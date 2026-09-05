//! Harbor domain-process entry: framed stdio over the in-process core.
//!
//! All state and dispatch live in [`harbor_core::app`], shared verbatim
//! with the mobile build (which drives the same core through its C ABI).
//! This binary only owns the stdio transport to the Qt facade.

fn main() {
    if let Err(error) = harbor_core::app::run_stdio() {
        eprintln!("harbor-core: {error}");
        std::process::exit(1);
    }
}
