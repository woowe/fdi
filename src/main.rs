/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::io;
use std::io::StdoutLock;
use std::io::{stdin, stdout, Write};
use std::path::Path;

use termion::raw::IntoRawMode;
use tui::backend::TermionBackend;
use tui::layout::{Constraint, Direction, Layout};
use tui::widgets::{Block, Borders, Widget};
use tui::Terminal;

fn display_prompt(stdout: &mut StdoutLock, dir: &Path, input: &str) -> Result<(), Box<dyn Error>> {
    let absolute_path = std::fs::canonicalize(&dir)?;
    write!(
        stdout,
        "{}{}{}",
        termion::clear::All,
        termion::cursor::Goto(1, 1),
        absolute_path.to_string_lossy(),
    );

    Ok(())
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn Error>> {
    // get the current directory
    let mut dir = Path::new(".");

    let stdout = io::stdout().into_raw_mode()?;
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| {
            let size = f.size();
            let block = Block::default().title("Block").borders(Borders::ALL);
            f.render_widget(block, size);
        });
    }

    Ok(())
}
