//! Subprocess runner and parallel job execution for `sisho-board`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use jobserver_gate::JobServerGate;

static GATE: OnceLock<JobServerGate> = OnceLock::new();

/// Initialize the process-wide jobserver gate.
pub fn init_gate(client: jobserver::Client) {
    let _ = GATE.set(JobServerGate::new(client));
}

fn gate() -> &'static JobServerGate {
    GATE.get_or_init(|| {
        let limit = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let client = jobserver::Client::new(limit.saturating_sub(1))
            .expect("failed to initialize fallback jobserver");
        JobServerGate::new(client)
    })
}

/// A command ready to be run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    name: String,
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    current_dir: Option<PathBuf>,
}

impl Step {
    /// A step called `name` that runs `program` with no arguments yet.
    pub fn new(name: impl Into<String>, program: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
            current_dir: None,
        }
    }

    /// Add one argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add several arguments in order.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable for the child process.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Set several environment variables for the child process.
    pub fn envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        for (key, value) in envs {
            self.envs.push((key.into(), value.into()));
        }
        self
    }

    /// Run in `dir` instead of the current working directory.
    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// The short name given at construction.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The program and arguments joined by spaces, for display.
    pub fn command_line(&self) -> String {
        let mut parts = Vec::with_capacity(1 + self.args.len());
        parts.push(self.program.to_string_lossy().into_owned());
        for arg in &self.args {
            parts.push(arg.to_string_lossy().into_owned());
        }
        parts.join(" ")
    }

    /// Run the command with standard input closed and standard output and
    /// standard error captured into an [`Outcome`].
    pub fn run(&self) -> Outcome {
        let start = Instant::now();
        let output = self
            .command()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let duration = start.elapsed();

        let (status, stdout, stderr) = match output {
            Ok(out) => (
                Status::from_exit(out.status),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ),
            Err(err) => (Status::from_io_error(&err), String::new(), String::new()),
        };

        Outcome {
            name: self.name.clone(),
            command_line: self.command_line(),
            status,
            stdout,
            stderr,
            duration,
        }
    }

    /// Run the command with standard input, output and error inherited from
    /// this process.
    pub fn run_inherited(&self) -> Status {
        match self
            .command()
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
        {
            Ok(exit) => Status::from_exit(exit),
            Err(err) => Status::from_io_error(&err),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.envs {
            command.env(key, value);
        }
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        gate().configure(&mut command);
        command
    }
}

/// How a step ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Exited with code 0.
    Passed,
    /// Exited with a non-zero code.
    Failed { code: i32 },
    /// Ended by a signal before it could exit.
    Signalled { signal: i32 },
    /// The program was not found on `PATH` (`ENOENT`).
    NotFound,
    /// The process could not be started for another reason.
    CouldNotStart { reason: String },
}

impl Status {
    /// True only for [`Status::Passed`].
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Passed)
    }

    fn from_exit(exit: std::process::ExitStatus) -> Self {
        match exit.code() {
            Some(0) => Self::Passed,
            Some(code) => Self::Failed { code },
            None => Self::Signalled {
                signal: signal_of(exit),
            },
        }
    }

    fn from_io_error(err: &io::Error) -> Self {
        if err.kind() == io::ErrorKind::NotFound {
            Self::NotFound
        } else {
            Self::CouldNotStart {
                reason: err.to_string(),
            }
        }
    }
}

#[cfg(unix)]
fn signal_of(exit: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    exit.signal().unwrap_or(0)
}

#[cfg(not(unix))]
fn signal_of(_exit: std::process::ExitStatus) -> i32 {
    0
}

/// What a single step produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Short name of the step, e.g. `"tomet check"`.
    pub name: String,
    /// The command line that was run.
    pub command_line: String,
    /// How the step ended.
    pub status: Status,
    /// Standard output, decoded as UTF-8 with replacement.
    pub stdout: String,
    /// Standard error, decoded as UTF-8 with replacement.
    pub stderr: String,
    /// Wall-clock time from start to finish.
    pub duration: Duration,
}

impl Outcome {
    /// Build an outcome from work done inside `sisho-board` rather than a subprocess.
    pub fn in_process(
        name: impl Into<String>,
        status: Status,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        duration: Duration,
    ) -> Self {
        let name = name.into();
        Self {
            command_line: name.clone(),
            name,
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
            duration,
        }
    }

    /// True if [`Outcome::status`] is [`Status::Passed`].
    pub fn is_pass(&self) -> bool {
        self.status.is_pass()
    }
}

/// Run `steps` in parallel and invoke `on_complete(index, outcome)` as each step finishes.
/// Returns the outcomes in their original plan order.
pub fn run_parallel_streaming<F>(steps: Vec<Step>, on_complete: F) -> Vec<Outcome>
where
    F: Fn(usize, Outcome) + Send + Sync,
{
    let gate = gate();
    let on_complete = &on_complete;
    let results: Vec<Outcome> = std::thread::scope(|scope| {
        let handles: Vec<_> = steps
            .into_iter()
            .enumerate()
            .map(|(index, step)| {
                scope.spawn(move || {
                    let _token = gate.acquire();
                    let outcome = step.run();
                    on_complete(index, outcome.clone());
                    outcome
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("step thread panicked"))
            .collect()
    });
    gate.reclaim_implicit_token();
    results
}

/// Run a program with arguments and a timeout, returning `Some(stdout)` if it exits 0 within `timeout`.
pub fn run_with_timeout<I, S>(
    program: &Path,
    args: I,
    timeout: Duration,
) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_with_timeout_env(
        program,
        args,
        std::iter::empty::<(&OsStr, &OsStr)>(),
        timeout,
    )
}

/// Run a program with arguments, additional environment variables, and a timeout,
/// returning `Some(stdout)` if it exits 0 within `timeout`.
pub fn run_with_timeout_env<I, S, E, K, V>(
    program: &Path,
    args: I,
    envs: E,
    timeout: Duration,
) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    const POLL_INTERVAL: Duration = Duration::from_millis(5);

    let mut command = Command::new(program);
    command
        .args(args)
        .envs(envs)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    gate().configure(&mut command);

    let mut child = command.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut stdout_buf = Vec::new();
                if let Some(mut stream) = child.stdout.take() {
                    use std::io::Read;
                    let _ = stream.read_to_end(&mut stdout_buf);
                }
                return String::from_utf8(stdout_buf).ok();
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}
