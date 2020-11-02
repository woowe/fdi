/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::path::Path;

use pancurses::{endwin, initscr, noecho, Input, Window};

fn display_prompt(window: &Window, dir: &Path, input: &str) -> Result<(), Box<dyn Error>> {
    let absolute_path = std::fs::canonicalize(&dir)?;

    let prompt = format!("{} {}", absolute_path.to_string_lossy(), input);
    window.mvprintw(0, 0, &prompt);

    Ok(())
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn Error>> {
    // get the current directory
    let mut dir = Path::new(".");

    let mut input = String::new();

    // setup the ncurses screen
    let window = initscr();
    display_prompt(&window, dir, "")?;
    window.refresh();
    window.keypad(true);
    noecho();

    loop {
        match window.getch() {
            Some(Input::Character(c)) => {
                input.push(c);
                eprintln!("{:?}", input);
            }
            Some(Input::KeyDC) => break,
            Some(Input::KeyBackspace) => {
                if input.len() < 1 {
                    input.clear();
                } else {
                    input = input.chars().take(input.len() - 1).collect::<String>();
                }

                eprintln!("{:?}", input);
                window.clear();
            }
            Some(input) => {
                eprintln!("{:?}", input);
            }
            None => (),
        }

        display_prompt(&window, dir, &input)?;
        window.refresh();

        // display the current promp
        // display_prompt(&window, dir, &input)?;
        // window.refresh();
    }

    endwin();

    Ok(())
}
