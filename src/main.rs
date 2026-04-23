use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

mod app;
mod build_info;
mod format;
mod game;
mod i18n;
mod save;
mod self_cmd;
mod ui;

use app::App;

#[derive(Parser)]
#[command(
    name = "cuqueclicker",
    version,
    about = "A TUI idle clicker where you finger an ASCII ass instead of clicking a cookie."
)]
struct Cli {
    /// Disable debug mode — hides the overlay and disables the F-key
    /// cheats (F1-F4). No effect in release builds, debug mode is never
    /// active there regardless of this flag.
    #[arg(long)]
    no_debug: bool,

    /// Start a self-playing demo on a pre-loaded rich state for
    /// asciinema/SVG capture, optionally taking a duration in seconds.
    /// Example: `--demo-for-recording 45`. Default 30s. Ignored in release
    /// builds. Hidden from help.
    #[arg(long, num_args = 0..=1, default_missing_value = "30", hide = true, value_name = "SECONDS")]
    demo_for_recording: Option<u32>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage this CuqueClicker installation.
    #[command(name = "self", subcommand)]
    Self_(SelfAction),
}

#[derive(Subcommand)]
enum SelfAction {
    /// Update the binary to the latest released version.
    Update,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Subcommands run in a plain stdio shell context — no terminal setup,
    // no save lock, no game loop.
    if let Some(Command::Self_(action)) = &cli.command {
        return match action {
            SelfAction::Update => self_cmd::update(),
        };
    }

    install_panic_hook();
    i18n::init();

    // Debug mode is on by default in dev builds, opt-out via --no-debug.
    // Release builds never show debug affordances, regardless of the flag.
    let debug = build_info::is_dev_build() && !cli.no_debug;

    // Demo mode (dev only) runs on an ephemeral rich state for asciinema
    // recording. It never acquires the save lock, never reads the user's
    // save, and never writes anything back — launching alongside a live
    // game session is safe.
    let demo_seconds = cli
        .demo_for_recording
        .filter(|_| build_info::is_dev_build());

    let _lock;
    let state = if demo_seconds.is_some() {
        app::build_demo_state()
    } else {
        _lock = match save::acquire_lock() {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "CuqueClicker is already running (or the save directory is not writable)."
                );
                eprintln!("Close the other instance and try again.");
                eprintln!("Details: {e}");
                std::process::exit(1);
            }
        };
        save::load()
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = App::new(state, debug, demo_seconds).run(&mut terminal);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
