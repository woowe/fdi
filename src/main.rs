/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::io::{stdout, Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use pancurses::{endwin, initscr, noecho, Input, Window};
use tokio::io::Lines;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::process::Command;

use termion::input::TermRead;
use termion::raw::IntoRawMode;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdout = stdout();
    let mut stdout = stdout.lock().into_raw_mode().unwrap();
    let mut stdin = termion::async_stdin().bytes();

    write!(
        stdout,
        "{}{}",
        termion::clear::All,
        termion::cursor::Goto(1, 1)
    )
    .unwrap();

    loop {
        let b = stdin.next();

        if let Some(ch) = b {
            write!(stdout, "\r{:?}    <- This demonstrates the async read input char. Between each update a 100 ms. is waited, simply to demonstrate the async fashion. \n\r", ch).unwrap();
            if let Ok(b'q') = ch {
                break;
            }
        }

        write!(stdout, "{}", termion::cursor::Goto(1, 1)).unwrap();
        stdout.flush().unwrap();
    }

    Ok(())
}
