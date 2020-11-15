/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::io::{stdout, StdoutLock, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::spawn;
use std::time::Duration;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use termion::color;
use termion::event::Key;
use termion::input::Keys;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::raw::RawTerminal;
use tokio::io::Lines;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::process::Command;

//
// At a high level we want 4 threads
// 1. the main thread for displaying output
// 2. one to get the current keys
// 3. one to run the command
// 4. one to sort the output of the command

// channel structure:
// main <- input keys
//      <- sort output <- command

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

    pub fn display(&self, term_width: usize) -> String {
        let mut line = self
            .data
            .char_indices()
            .take(term_width)
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

        line.push('\n');

        line
    }
}

async fn spawn_fd(dir: &PathBuf) -> Result<Lines<BufReader<ChildStdout>>, Box<dyn Error>> {
    let mut cmd = Command::new("fd");

    cmd.arg("-H");
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

enum AppEvent {
    Quit,
    Input((PathBuf, String)),
    Dir((PathBuf, String)),
    Sorted(Vec<String>),
    Unknown,
}

fn handle_keys(tx: Sender<AppEvent>) {
    let mut stdin = termion::async_stdin().keys();
    let mut input = String::new();
    let mut dir = Path::new(".").canonicalize().unwrap();

    // send the inital data
    let _ = tx.send(AppEvent::Input((dir.clone(), input.clone())));

    loop {
        let key = stdin.next();

        // handle the keys
        if let Some(key) = key {
            // match on the event sent from stdin
            if let Ok(key) = key {
                match key {
                    Key::Ctrl('c') => {
                        let _ = tx.send(AppEvent::Quit);
                    }
                    Key::Backspace => {
                        if input.len() < 1 {
                            if let Some(parent) = dir.parent() {
                                dir = parent.to_path_buf();
                                let _ = tx.send(AppEvent::Dir((dir.clone(), input.clone())));
                            }
                        } else {
                            input = input[0..input.len() - 1].to_string();
                            let _ = tx.send(AppEvent::Input((dir.clone(), input.clone())));
                        }
                    }
                    Key::Char('\n') | Key::Char('\t') => {
                        if let Ok(input_dir) = dir.join(&input).canonicalize() {
                            dir = input_dir;

                            input.clear();

                            let _ = tx.send(AppEvent::Dir((dir.clone(), input.clone())));
                        }
                    }
                    Key::Char(ch) => {
                        // dont care about poisoning
                        input.push(ch);
                        let _ = tx.send(AppEvent::Input((dir.clone(), input.clone())));
                    }
                    _ => {
                        let _ = tx.send(AppEvent::Unknown);
                    }
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let stdout = stdout();
    let mut stdout = stdout.lock().into_raw_mode().unwrap();

    // spawn fd
    // this read will async. read the lines
    // from stdout
    // let mut reader = spawn_fd(&dir).await?;
    // we want to record the lines in a vector
    // so we can do fuzzy searching over it
    let mut output: Vec<OutputLine> = Vec::new();
    // get the term height so we don't display more
    // output than we need
    let (term_width, term_height) = termion::terminal_size()?;
    eprintln!("{}, {}", term_width, term_height);
    let output_offset = 3u16;
    // just for knowing what the user has typed
    let matcher = SkimMatcherV2::default();

    clear_screen(&mut stdout)?;

    let (mut tx, mut rx) = channel();

    spawn(move || {
        handle_keys(tx.clone());
    });

    for event in rx.iter() {
        match event {
            AppEvent::Quit => {
                break;
            }
            AppEvent::Dir((dir, input)) => {
                // prompt
                write!(
                    stdout,
                    "{}{} > {} {}",
                    termion::clear::CurrentLine,
                    termion::cursor::Goto(1, 1),
                    dir.to_string_lossy(),
                    input
                )?;
                stdout.flush()?;
            }
            AppEvent::Input((dir, input)) => {
                // prompt
                write!(
                    stdout,
                    "{}{} > {} {}",
                    termion::clear::CurrentLine,
                    termion::cursor::Goto(1, 1),
                    dir.to_string_lossy(),
                    input
                )?;
                stdout.flush()?;
            }
            _ => {
                eprintln!("Uknown event");
            }
        }
    }

    // 'main: loop {
    //     // Select the next line from the fd output
    //     // and store it into an output buffer
    //     tokio::select! {
    //         line = reader.next_line() => {
    //             if let Ok(line) = line {
    //                 if let Some(line) = line {
    //                     output.push(OutputLine::new(line, &matcher, &input));
    //                     output.sort_by(|a, b| b.score.cmp(&a.score));
    //                 }
    //             }
    //         }
    //     }

    //     // handle the keys
    //     if let Some(key) = key {
    //         // match on the event sent from stdin
    //         if let Ok(key) = key {
    //             match key {
    //                 // break when ctrl + c is pressed
    //                 Key::Ctrl('c') => {
    //                     break 'main;
    //                 }
    //                 // try to change directories on enter
    //                 Key::Char('\n') => {
    //                     if let Ok(input_dir) = dir.join(&input).canonicalize() {
    //                         dir = input_dir;

    //                         input.clear();
    //                         output.clear();
    //                         reader = spawn_fd(&dir).await?;

    //                         clear_screen(&mut stdout)?;
    //                     }
    //                 }
    //                 // handle keyboard input
    //                 Key::Char(ch) => {
    //                     let exclude = exclude_chars.iter().find(|&ex| *ex == ch);

    //                     if exclude.is_none() {
    //                         input.push(ch);
    //                         update_fuzz(&mut output, &matcher, &input);
    //                         clear_screen(&mut stdout)?;
    //                     }
    //                 }
    //                 // handle the backspace
    //                 Key::Backspace => {
    //                     if input.len() < 1 {
    //                         input.clear();

    //                         // go up to the parent directory
    //                         if let Some(parent_dir) = dir.parent() {
    //                             dir = PathBuf::from(parent_dir);
    //                             output.clear();
    //                             reader = spawn_fd(&dir).await?;
    //                         }
    //                     } else {
    //                         input = input.chars().take(input.len() - 1).collect::<String>();
    //                         update_fuzz(&mut output, &matcher, &input);
    //                     }

    //                     // Make sure the screen gets a full clear when the backspace happens
    //                     clear_screen(&mut stdout)?;
    //                 }
    //                 _ => {}
    //             }
    //         }
    //     }

    //     // output the up to the term height of
    //     // lines from the command output
    //     let cmd_output = output
    //         .iter()
    //         .take(term_height as usize - (output_offset + 0) as usize)
    //         .map(|line| line.display(term_width as usize))
    //         .collect::<Vec<String>>()
    //         .join("\r");

    //     write!(
    //         stdout,
    //         "{}{}{}",
    //         termion::cursor::Goto(1, output_offset),
    //         cmd_output,
    //         color::Fg(color::Reset)
    //     )?;

    //     // progress indicator of sorts
    //     let total = output.len();
    //     let results = output.len();
    //     write!(
    //         stdout,
    //         "{} {}/{}",
    //         termion::cursor::Goto(1, 2),
    //         results,
    //         total
    //     )?;

    //     // prompt
    //     write!(
    //         stdout,
    //         "{} > {} {}",
    //         termion::cursor::Goto(1, 1),
    //         dir.to_string_lossy(),
    //         input,
    //     )?;
    //     stdout.flush()?;

    //     std::thread::sleep(Duration::from_millis(3));
    // }

    Ok(())
}
