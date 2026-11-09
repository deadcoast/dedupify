use std::process::ExitCode;

use dedupe::actions;
use dedupe::cli::{self, Command};
use dedupe::doctor;
use dedupe::help;
use dedupe::menu;
use dedupe::render::Renderer;
use dedupe::theme::Theme;

fn main() -> ExitCode {
    // before any worker thread exists: the local-date offset for keep lines
    dedupe::util::init_local_offset();
    let theme = Theme::detect();
    let r = Renderer::new(theme);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Ctrl-C ends a scan at once (with a clean newline after the in-place
    // progress line) and, once a change has started, stops it after the
    // file in hand with the journal written — see interrupt.rs
    dedupe::interrupt::install();
    let code = run(&args, &cwd, &r, theme);
    ExitCode::from(code as u8)
}

fn run(args: &[String], cwd: &std::path::Path, r: &Renderer, theme: Theme) -> i32 {
    let mut inv = match cli::parse(args, cwd) {
        Ok(i) => i,
        Err(e) => {
            r.error(&e.to_string());
            return actions::EXIT_USER;
        }
    };
    // the skip list in force: remembered, plus --skip, minus --unskip
    if matches!(
        inv.command,
        Command::Find | Command::Milk | Command::Purge | Command::Doctor
    ) {
        inv.skip = dedupe::skip::resolve(&inv.skip_add, &inv.skip_remove);
    }

    match inv.command {
        Command::Help => {
            help::page(r);
            actions::EXIT_OK
        }
        Command::Menu => {
            if !theme.live {
                // nothing to steer with; show the page instead
                help::page(r);
                return actions::EXIT_OK;
            }
            match menu::run(theme, &cwd.to_string_lossy()) {
                Ok(Some(menu_args)) => run(&menu_args, cwd, r, theme),
                Ok(None) => actions::EXIT_OK,
                Err(e) => {
                    r.error(&format!("menu unavailable  {e}"));
                    help::page(r);
                    actions::EXIT_USER
                }
            }
        }
        Command::Doctor => doctor::page(&inv, r),
        _ => actions::run(&inv, r),
    }
}
