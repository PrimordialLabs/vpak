//! Spawn a child, feed it stdin, stream its stdout line by line, and kill it
//! when a wall-clock budget expires. No shell, no signals.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

#[derive(Debug, Default)]
pub struct Finished {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stderr_tail: String,
}

/// Run `cmd`, writing `stdin_data` to the child, calling `on_line` for every
/// stdout line, and killing the child if `deadline` passes.
pub fn run_streaming(
    cmd: &mut Command,
    stdin_data: Option<&str>,
    deadline: Option<Duration>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Finished> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {:?}", cmd.get_program()))?;

    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            let data = data.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(data.as_bytes());
                let _ = stdin.flush();
            });
        }
    }

    let stdout = child.stdout.take().context("child stdout")?;
    let stderr = child.stderr.take().context("child stderr")?;
    let (tx, rx) = mpsc::channel::<String>();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let err_thread = std::thread::spawn(move || {
        let mut tail: Vec<String> = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push(line);
            if tail.len() > 40 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });

    let start = Instant::now();
    let mut timed_out = false;
    let exit_code;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => on_line(line.trim_end_matches('\r')),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout closed: wait for exit.
                let status = child.wait()?;
                exit_code = status.code();
                break;
            }
        }
        if let Some(d) = deadline {
            if start.elapsed() > d {
                timed_out = true;
                let _ = child.kill();
                let status = child.wait()?;
                exit_code = status.code();
                break;
            }
        }
        if let Some(status) = child.try_wait()? {
            // Drain anything left, then stop.
            while let Ok(line) = rx.try_recv() {
                on_line(line.trim_end_matches('\r'));
            }
            exit_code = status.code();
            break;
        }
    }
    let _ = out_thread.join();
    // Deliver any lines that arrived between the last poll and thread exit.
    while let Ok(line) = rx.try_recv() {
        on_line(line.trim_end_matches('\r'));
    }
    let stderr_tail = err_thread.join().unwrap_or_default();
    Ok(Finished {
        exit_code,
        timed_out,
        stderr_tail,
    })
}
