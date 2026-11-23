use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use sisho_sdk::{Capabilities, ExtensionInfo};

/// The interactive operating dashboard and board for the Tomet ecosystem:
/// inspect core tool versions, extension status and trust permissions,
/// extension board panes, integrity checks, and broken links.
#[derive(Parser)]
#[command(
    name = "sisho-board",
    version,
    about = "Interactive operating dashboard and board for the Tomet ecosystem"
)]
struct Cli {
    /// Disclose extension metadata for sisho.
    #[arg(long = "sisho-extension-info", hide = true)]
    sisho_extension_info: bool,

    /// Project or vault directory to inspect (defaults to ".").
    #[arg(default_value = ".")]
    path: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.sisho_extension_info {
        let info = ExtensionInfo::new(
            "board",
            env!("CARGO_PKG_VERSION"),
            "Interactive operating dashboard and board for the Tomet ecosystem",
        )
        .with_capabilities(Capabilities::new().with_tui(true).with_interactive(true));
        if let Err(error) = info.print() {
            eprintln!("sisho-board: failed to write extension info: {error}");
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    let _ = init_jobserver();

    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("sisho-board: standard input and output must be a terminal");
        return ExitCode::from(1);
    }

    match sisho_board_core::run(cli.path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sisho-board: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn init_jobserver() -> std::io::Result<()> {
    // SAFETY: `from_env_ext` reads and takes ownership of file descriptors specified in
    // environment variables (such as `CARGO_MAKEFLAGS`). This function is called once,
    // early in `main` before any other file descriptors are opened or threads are spawned.
    let from_env = unsafe { jobserver::Client::from_env_ext(true) };
    let client = match from_env.client {
        Ok(client) => client,
        Err(_) => {
            let limit = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            jobserver::Client::new(limit.saturating_sub(1))?
        }
    };
    sisho_board_core::runner::init_gate(client);
    Ok(())
}
