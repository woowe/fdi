/// Trying to make an interactive version of fd
/// much like fzf but with the specific purpose to navigate
/// the filesystem
use std::error::Error;
use std::io::{stdout, StdoutLock, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
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

async fn spawn_fd(
    dir: &PathBuf,
    output: &Arc<Mutex<Vec<OutputLine>>>,
    pattern: &Arc<Mutex<String>>,
    matcher: &Arc<Mutex<SkimMatcherV2>>,
) {
    let out = Arc::clone(output);
    let dir = dir.clone();
    let pattern = Arc::clone(pattern);
    let matcher = Arc::clone(matcher);

    {
        let mut lock = out.lock().expect("Unable to get lock");
        lock.clear();
    }

    tokio::spawn(async move {
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

        let mut reader = BufReader::new(stdout).lines();

        tokio::spawn(async move {
            let status = child
                .wait()
                .await
                .expect("child process encountered an error");

            eprintln!("child status was: {}", status);
        });

        while let Some(line) = reader.next_line().await.expect("Unable to get next line") {
            let mut out = out.lock().expect("Unable to obtain lock");
            let pattern = pattern.lock().expect("Unable to obtain lock");
            let matcher = matcher.lock().expect("Unable to obtain lock");
            out.push(OutputLine::new(line, &matcher, &pattern));
        }
    });
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

    // we want to record the lines in a vector
    // so we can do fuzzy searching over it
    let output: Arc<Mutex<Vec<OutputLine>>> = Arc::new(Mutex::new(Vec::new()));
    // just for knowing what the user has typed
    let input = Arc::new(Mutex::new(String::new()));
    let matcher = Arc::new(Mutex::new(SkimMatcherV2::default()));
    // spawn fd
    // this read will async. read the lines
    // from stdout
    spawn_fd(&dir, &output, &input, &matcher).await;
    // get the term height so we don't display more
    // output than we need
    let (term_width, term_height) = termion::terminal_size()?;
    let output_offset = 3u16;
    let exclude_chars = vec!['\n', '\t'];

    clear_screen(&mut stdout)?;

    'main: loop {
        let key = stdin.next();

        // handle the keys
        if let Some(key) = key {
            // match on the event sent from stdin
            if let Ok(key) = key {
                match key {
                    // break when ctrl + c is pressed
                    Key::Ctrl('c') => {
                        break 'main;
                    }
                    // try to change directories on enter
                    Key::Char('\n') => {
                        let mut input_l = input.lock().unwrap();
                        if let Ok(input_dir) = dir.join(&*input_l).canonicalize() {
                            dir = input_dir;

                            input_l.clear();
                            spawn_fd(&dir, &output, &input, &matcher).await;

                            clear_screen(&mut stdout)?;
                        }
                    }
                    // handle keyboard input
                    Key::Char(ch) => {
                        let exclude = exclude_chars.iter().find(|&ex| *ex == ch);

                        if exclude.is_none() {
                            let mut input = input.lock().unwrap();
                            let mut output = output.lock().unwrap();
                            let matcher = matcher.lock().unwrap();

                            input.push(ch);
                            update_fuzz(&mut output, &matcher, &input);
                            clear_screen(&mut stdout)?;
                        }
                    }
                    // handle the backspace
                    Key::Backspace => {
                        let mut input_l = input.lock().unwrap();
                        if input_l.len() < 1 {
                            input_l.clear();

                            // go up to the parent directory
                            if let Some(parent_dir) = dir.parent() {
                                dir = PathBuf::from(parent_dir);
                                spawn_fd(&dir, &output, &input, &matcher).await;
                            }
                        } else {
                            *input_l = input_l.chars().take(input_l.len() - 1).collect::<String>();
                            let mut output = output.lock().unwrap();
                            let matcher = matcher.lock().unwrap();

                            update_fuzz(&mut output, &matcher, &input_l);
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
        {
            let mut output = output.lock().unwrap();
            let input = input.lock().unwrap();
            let matcher = matcher.lock().unwrap();

            // update_fuzz(&mut output, &matcher, &input);

            let cmd_output = output
                .iter()
                .take(term_height as usize - (output_offset + 0) as usize)
                .map(|line| line.display(term_width as usize))
                .collect::<Vec<String>>()
                .join("\r");

            write!(
                stdout,
                "{}{}{}",
                termion::cursor::Goto(1, output_offset),
                cmd_output,
                color::Fg(color::Reset)
            )?;

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
        }

        // prompt
        {
            let input = input.lock().unwrap();
            write!(
                stdout,
                "{} > {} {}",
                termion::cursor::Goto(1, 1),
                dir.to_string_lossy(),
                input,
            )?;
        }
        stdout.flush()?;

        std::thread::sleep(Duration::from_millis(3));
    }

    Ok(())
}
