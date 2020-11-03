/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::io::{stdout, StdoutLock, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use termion::color;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::raw::RawTerminal;
use tokio::io::Lines;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::process::Command;

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
struct OutputLine {
    data: String,
    score: i64,
    indices: Vec<usize>,
}

impl OutputLine {
    pub fn new(data: String, matcher: &SkimMatcherV2, match_with: &str) -> OutputLine {
        let mut score: i64 = Default::default();
        let mut indices: Vec<usize> = Default::default();

        if let Some((fscore, findices)) = matcher.fuzzy_indices(&data, &match_with) {
            score = fscore;
            indices = findices;

            eprintln!("line -- {}, score: {}", data, score);
        }

        OutputLine {
            data,
            score,
            indices,
        }
    }

    pub fn update(&mut self, matcher: &SkimMatcherV2, match_with: &str) -> &mut OutputLine {
        if let Some((fscore, findices)) = matcher.fuzzy_indices(&self.data, &match_with) {
            self.score = fscore;
            self.indices = findices;
        }

        self
    }

    pub fn display(
        &self,
        stdout: &mut RawTerminal<StdoutLock>,
        x: u16,
        y: u16,
    ) -> Result<(), Box<dyn Error>> {
        let line = self
            .data
            .char_indices()
            .map(move |(i, ch)| {
                let found = self.indices.iter().find(|&idx| *idx == i);

                if found.is_some() {
                    // color the character
                    format!("{}{}", color::Fg(color::Red), ch)
                } else {
                    format!("{}{}", color::Fg(color::Reset), ch)
                }
            })
            .collect::<Vec<String>>()
            .join("");

        write!(
            stdout,
            "{}{}{}",
            termion::cursor::Goto(x, y),
            line,
            color::Fg(color::Reset)
        )?;

        Ok(())
    }
}

async fn spawn_fd(dir: &PathBuf) -> Result<Lines<BufReader<ChildStdout>>, Box<dyn Error>> {
    let mut cmd = Command::new("fd");

    // TODO: get rid of this when not testing
    // cmd.arg("-I");
    cmd.current_dir(dir);

    // pipe fd stdout to the programs stdout
    cmd.stdout(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn command");

    let stdout = child
        .stdout
        .take()
        .expect("child did not have a handle to stdout");

    let reader = BufReader::new(stdout).lines();

    tokio::spawn(async move {
        let status = child
            .wait()
            .await
            .expect("child process encountered an error");

        eprintln!("child status was: {}", status);
    });

    Ok(reader)
}

fn clear_screen(stdout: &mut RawTerminal<StdoutLock>) -> Result<(), Box<dyn Error>> {
    write!(
        stdout,
        "{}{}",
        termion::clear::All,
        termion::cursor::Goto(1, 1)
    )?;
    stdout.flush()?;

    Ok(())
}

fn update_fuzz(output: &mut Vec<OutputLine>, matcher: &SkimMatcherV2, pattern: &str) {
    for line in output.iter_mut() {
        line.update(matcher, pattern);
    }

    output.sort_by(|a, b| b.score.cmp(&a.score));
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let stdout = stdout();
    let mut stdout = stdout.lock().into_raw_mode().unwrap();
    let mut stdin = termion::async_stdin().keys();

    let mut dir = Path::new(".").canonicalize()?;

    // spawn fd
    // this read will async. read the lines
    // from stdout
    let mut reader = spawn_fd(&dir).await?;
    // we want to record the lines in a vector
    // so we can do fuzzy searching over it
    let mut output: Vec<OutputLine> = Vec::new();
    // get the term height so we don't display more
    // output than we need
    let (_, term_height) = termion::terminal_size()?;
    let output_offset = 3u16;
    // just for knowing what the user has typed
    let mut input = String::new();

    let matcher = SkimMatcherV2::default();

    let exclude_chars = vec!['\n', '\t'];

    clear_screen(&mut stdout)?;

    'main: loop {
        let key = stdin.next();

        // Select the next line from the fd output
        // and store it into an output buffer
        tokio::select! {
            line = reader.next_line() => {
                if let Ok(line) = line {
                    if let Some(line) = line {
                        output.push(OutputLine::new(line, &matcher, &input));
                        output.sort_by(|a, b| b.score.cmp(&a.score));
                    }
                }
            }
        }

        // handle the keys
        if let Some(key) = key {
            // match on the event sent from stdin
            if let Ok(key) = key {
                match key {
                    // break when the 'q' character is pressed
                    Key::Char('q') => {
                        break 'main;
                    }
                    // break when ctrl + c is pressed
                    Key::Ctrl('c') => {
                        break 'main;
                    }
                    // try to change directories on enter
                    Key::Char('\n') => {
                        if let Ok(input_dir) = dir.join(&input).canonicalize() {
                            dir = input_dir;

                            input.clear();
                            output.clear();
                            reader = spawn_fd(&dir).await?;

                            clear_screen(&mut stdout)?;
                        }
                    }
                    // handle keyboard input
                    Key::Char(ch) => {
                        let exclude = exclude_chars.iter().find(|&ex| *ex == ch);

                        if exclude.is_none() {
                            input.push(ch);
                            update_fuzz(&mut output, &matcher, &input);
                            clear_screen(&mut stdout)?;
                        }
                    }
                    // handle the backspace
                    Key::Backspace => {
                        if input.len() < 1 {
                            input.clear();

                            // go up to the parent directory
                            if let Some(parent_dir) = dir.parent() {
                                dir = PathBuf::from(parent_dir);
                                output.clear();
                                reader = spawn_fd(&dir).await?;
                            }
                        } else {
                            input = input.chars().take(input.len() - 1).collect::<String>();
                            update_fuzz(&mut output, &matcher, &input);
                        }

                        // Make sure the screen gets a full clear when the backspace happens
                        clear_screen(&mut stdout)?;
                    }
                    _ => {}
                }
            }
        }

        // output the up to the term height of
        // lines from the command output
        for (i, line) in output.iter().take(term_height as usize).enumerate() {
            line.display(&mut stdout, 1, i as u16 + output_offset)?;
        }

        // progress indicator of sorts
        let total = output.len();
        let results = output.len();
        write!(
            stdout,
            "{} {}/{}",
            termion::cursor::Goto(1, 2),
            results,
            total
        )?;

        // prompt
        write!(
            stdout,
            "{} > {} {}",
            termion::cursor::Goto(1, 1),
            dir.to_string_lossy(),
            input,
        )?;
        stdout.flush()?;

        std::thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}
