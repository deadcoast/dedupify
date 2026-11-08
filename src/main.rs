use std::process::ExitCode;

use dedupe::actions;
use dedupe::cli::{self, Command};
use dedupe::doctor;
use dedupe::help;
use dedupe::menu;
use dedupe::render::Renderer;
use dedupe::theme::Theme;

fn main() -> ExitCode {
    let theme = Theme::detect();
    let r = Renderer::new(theme);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let args: Vec<String> = std::env::args().skip(1).collect();

    let code = run(&args, &cwd, &r, theme);
    ExitCode::from(code as u8)
}

fn run(args: &[String], cwd: &std::path::Path, r: &Renderer, theme: Theme) -> i32 {
    let inv = match cli::parse(args, cwd) {
        Ok(i) => i,
        Err(e) => {
            r.error(&e.to_string());
            return actions::EXIT_USER;
        }
    };

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
