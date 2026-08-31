//! The binary, holding nothing, so that everything `bdi` reaches can be
//! private to the library.
//!
//! bdi's command line lives in the library, as `cli`. A binary is a separate
//! crate, and a module it names has to be `pub` for it — which is what
//! switched `dead_code` off across `src/` for as long as this file held the
//! command line itself.

fn main() -> anyhow::Result<std::process::ExitCode> {
    beady_eye::cli::run()
}
