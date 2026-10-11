//! Run-session supervisor (I-ST10). Run sessions are owned by the host
//! process, never by a renderer connection: a session carries an id, a start
//! time, its node processes (pid, captured stdout/stderr, status), and the run
//! config whose `leave_running` flag is the disconnect policy. Closing Studio
//! with running nodes prompts Leave-running/Stop; a disconnect never orphans or
//! kills node processes (`REQ-STUDIO-010`).
//!
//! Process launching is a [`ProcessRunner`] seam so tests never spawn OS
//! processes; [`CommandRunner`] is the concrete runner used by the host.
//!
//! REQ: REQ-STUDIO-010 (run sessions outlive the Studio window).

use std::collections::BTreeMap;
use std::fmt;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::journal::fresh_id;
use super::protocol::{RunConfig, StudioError};

/// Lifecycle status of a supervised run session. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// At least one node process is owned by the session.
    Running,
    /// The session was stopped; no node process remains.
    Stopped,
}

/// Lifecycle status of a supervised node process. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    /// Spawned and running.
    Running,
    /// Exited on its own with the given code.
    Exited(i32),
    /// Terminated by the supervisor.
    Stopped,
}

/// A live node process handle owned by the supervisor. Implementations are
/// injectable so tests never spawn OS processes. REQ: REQ-STUDIO-010.
pub trait NodeHandle: Send + fmt::Debug {
    /// OS pid when the process was actually spawned. REQ: REQ-STUDIO-010.
    fn pid(&self) -> Option<u32>;
    /// Captured stdout so far. REQ: REQ-STUDIO-010.
    fn stdout(&self) -> String;
    /// Captured stderr so far. REQ: REQ-STUDIO-010.
    fn stderr(&self) -> String;
    /// Polls the process: `None` while running, exit code once exited.
    /// REQ: REQ-STUDIO-010.
    fn try_wait(&mut self) -> Option<i32>;
    /// Terminates the process (idempotent). REQ: REQ-STUDIO-010.
    fn kill(&mut self);
}

/// Spawns the node processes of a run session; injectable so tests never spawn
/// OS processes. REQ: REQ-STUDIO-010.
pub trait ProcessRunner: Send {
    /// Spawns `node` for `config`.
    ///
    /// # Errors
    /// [`StudioError::Io`] when the process cannot be spawned.
    fn spawn(&mut self, node: &str, config: &RunConfig)
    -> Result<Box<dyn NodeHandle>, StudioError>;
}

/// A supervised node process: its name, pid, status, and captured output.
/// REQ: REQ-STUDIO-010.
#[derive(Debug)]
pub struct NodeProcess {
    /// Node name.
    pub node: String,
    /// OS pid when the process was spawned.
    pub pid: Option<u32>,
    /// Current process status.
    pub status: ProcessStatus,
    handle: Box<dyn NodeHandle>,
}

impl NodeProcess {
    /// Wraps a freshly spawned handle as a running node process.
    /// REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn new(node: String, handle: Box<dyn NodeHandle>) -> Self {
        let pid = handle.pid();
        Self {
            node,
            pid,
            status: ProcessStatus::Running,
            handle,
        }
    }

    /// Captured stdout so far. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn stdout(&self) -> String {
        self.handle.stdout()
    }

    /// Captured stderr so far. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn stderr(&self) -> String {
        self.handle.stderr()
    }

    /// Refreshes a running process's status from the OS. REQ: REQ-STUDIO-010.
    pub fn refresh(&mut self) {
        if self.status == ProcessStatus::Running
            && let Some(code) = self.handle.try_wait()
        {
            self.status = ProcessStatus::Exited(code);
        }
    }

    /// Terminates the process if it is still running. REQ: REQ-STUDIO-010.
    pub fn stop(&mut self) {
        if self.status == ProcessStatus::Running {
            self.handle.kill();
            self.status = ProcessStatus::Stopped;
        }
    }

    /// Immutable wire-ready view of this process. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn view(&self) -> NodeProcessView {
        NodeProcessView {
            node: self.node.clone(),
            pid: self.pid,
            status: self.status,
            stdout: self.stdout(),
            stderr: self.stderr(),
        }
    }
}

/// A serializable snapshot of one node process. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProcessView {
    /// Node name.
    pub node: String,
    /// OS pid when spawned.
    pub pid: Option<u32>,
    /// Current status.
    pub status: ProcessStatus,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

/// A supervisor-owned run session. REQ: REQ-STUDIO-010.
#[derive(Debug)]
pub struct RunSession {
    /// Session id.
    pub id: String,
    /// Start time in Unix milliseconds.
    pub started_at: u64,
    /// Requested nodes + disconnect policy.
    pub run_config: RunConfig,
    /// Session status.
    pub status: SessionStatus,
    /// Supervised node processes.
    pub nodes: Vec<NodeProcess>,
}

impl RunSession {
    /// Refreshes every node's status. REQ: REQ-STUDIO-010.
    pub fn refresh_all(&mut self) {
        for node in &mut self.nodes {
            node.refresh();
        }
    }

    /// Stops every running node and marks the session stopped.
    /// REQ: REQ-STUDIO-010.
    pub fn stop_all(&mut self) {
        for node in &mut self.nodes {
            node.stop();
        }
        self.status = SessionStatus::Stopped;
    }

    /// Immutable wire-ready view of this session. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn view(&self) -> RunSessionView {
        RunSessionView {
            id: self.id.clone(),
            started_at: self.started_at,
            status: self.status,
            leave_running: self.run_config.leave_running,
            nodes: self.nodes.iter().map(NodeProcess::view).collect(),
        }
    }
}

/// A serializable snapshot of one run session. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSessionView {
    /// Session id.
    pub id: String,
    /// Start time in Unix milliseconds.
    pub started_at: u64,
    /// Session status.
    pub status: SessionStatus,
    /// Disconnect policy from the run config.
    pub leave_running: bool,
    /// Node process snapshots.
    pub nodes: Vec<NodeProcessView>,
}

/// Outcome of a Studio window-close request. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseDecision {
    /// No session is running; the window may close freely.
    Clear,
    /// Running sessions require the Leave-running/Stop prompt.
    Prompt {
        /// Ids of the sessions still running.
        sessions: Vec<String>,
    },
}

/// The operator's answer to the close prompt. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    /// Keep sessions running past the window (never kill).
    LeaveRunning,
    /// Stop every session and its node processes.
    Stop,
}

/// Owns run sessions independently of any UI lifetime. REQ: REQ-STUDIO-010.
pub struct Supervisor {
    runner: Box<dyn ProcessRunner>,
    sessions: BTreeMap<String, RunSession>,
}

impl fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Supervisor")
            .field("sessions", &self.sessions.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Creates an empty supervisor with a process runner. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn new(runner: Box<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            sessions: BTreeMap::new(),
        }
    }

    /// Starts a new session, spawning every requested node.
    /// REQ: REQ-STUDIO-010.
    ///
    /// # Errors
    /// [`StudioError::Io`] when a node process cannot be spawned; any node
    /// spawned before the failure is stopped, so no process is orphaned.
    pub fn start(&mut self, config: RunConfig) -> Result<String, StudioError> {
        let id = fresh_id();
        let nodes = self.spawn_nodes(&config)?;
        self.sessions.insert(
            id.clone(),
            RunSession {
                id: id.clone(),
                started_at: now_millis(),
                run_config: config,
                status: SessionStatus::Running,
                nodes,
            },
        );
        Ok(id)
    }

    /// Stops a session and every node process it owns. REQ: REQ-STUDIO-010.
    ///
    /// # Errors
    /// [`StudioError::Unknown`] when `id` is not a known session.
    pub fn stop(&mut self, id: &str) -> Result<RunSessionView, StudioError> {
        let session = self.session_mut(id)?;
        session.stop_all();
        Ok(session.view())
    }

    /// Restarts a session in place: new processes, same id and config.
    /// REQ: REQ-STUDIO-010.
    ///
    /// # Errors
    /// [`StudioError::Unknown`] for an unknown session; [`StudioError::Io`]
    /// when a replacement node process cannot be spawned.
    pub fn restart(&mut self, id: &str) -> Result<RunSessionView, StudioError> {
        let config = self
            .sessions
            .get(id)
            .map(|s| s.run_config.clone())
            .ok_or_else(|| unknown(id))?;
        self.stop(id)?;
        let nodes = self.spawn_nodes(&config)?;
        let session = self.session_mut(id)?;
        session.nodes = nodes;
        session.started_at = now_millis();
        session.status = SessionStatus::Running;
        Ok(session.view())
    }

    /// Handles a renderer disconnect (I-ST10): sessions whose `leave_running`
    /// is false are stopped, the rest keep running. Returns the ids still
    /// running. REQ: REQ-STUDIO-010.
    pub fn detach(&mut self) -> Vec<String> {
        let mut running = Vec::new();
        for (id, session) in &mut self.sessions {
            if session.status == SessionStatus::Stopped {
                continue;
            }
            if session.run_config.leave_running {
                running.push(id.clone());
            } else {
                session.stop_all();
            }
        }
        running
    }

    /// The close prompt decision for the current sessions. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn close_decision(&self) -> CloseDecision {
        let running: Vec<String> = self
            .sessions
            .values()
            .filter(|s| s.status == SessionStatus::Running)
            .map(|s| s.id.clone())
            .collect();
        if running.is_empty() {
            CloseDecision::Clear
        } else {
            CloseDecision::Prompt { sessions: running }
        }
    }

    /// Applies the operator's close choice. Returns the ids left running.
    /// REQ: REQ-STUDIO-010.
    pub fn apply_close(&mut self, choice: CloseChoice) -> Vec<String> {
        match choice {
            CloseChoice::LeaveRunning => self
                .sessions
                .values()
                .filter(|s| s.status == SessionStatus::Running)
                .map(|s| s.id.clone())
                .collect(),
            CloseChoice::Stop => {
                for session in self.sessions.values_mut() {
                    session.stop_all();
                }
                Vec::new()
            }
        }
    }

    /// Refreshes every session's node statuses. REQ: REQ-STUDIO-010.
    pub fn refresh(&mut self) {
        for session in self.sessions.values_mut() {
            session.refresh_all();
        }
    }

    /// A session snapshot by id. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn session(&self, id: &str) -> Option<RunSessionView> {
        self.sessions.get(id).map(RunSession::view)
    }

    /// Ids of every known session, sorted. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Spawns the node list for `config`, stopping partial work on failure.
    /// REQ: REQ-STUDIO-010.
    fn spawn_nodes(&mut self, config: &RunConfig) -> Result<Vec<NodeProcess>, StudioError> {
        let mut nodes: Vec<NodeProcess> = Vec::new();
        for name in &config.nodes {
            match self.runner.spawn(name, config) {
                Ok(handle) => nodes.push(NodeProcess::new(name.clone(), handle)),
                Err(err) => {
                    for node in &mut nodes {
                        node.stop();
                    }
                    return Err(err);
                }
            }
        }
        Ok(nodes)
    }

    /// Mutable session lookup. REQ: REQ-STUDIO-010.
    fn session_mut(&mut self, id: &str) -> Result<&mut RunSession, StudioError> {
        self.sessions.get_mut(id).ok_or_else(|| unknown(id))
    }
}

/// Builds an unknown-session error. REQ: REQ-STUDIO-010.
fn unknown(id: &str) -> StudioError {
    StudioError::Unknown(format!("unknown run session `{id}`"))
}

/// Current Unix time in milliseconds. REQ: REQ-STUDIO-010.
fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    u64::try_from(millis).unwrap_or(u64::MAX)
}

/// Concrete [`ProcessRunner`] that spawns a program per node, substituting
/// `{node}` in the program and argument templates and capturing stdout/stderr
/// on reader threads. REQ: REQ-STUDIO-010.
#[derive(Debug, Clone)]
pub struct CommandRunner {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl CommandRunner {
    /// Builds a runner for an explicit program + argument template.
    /// REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
        }
    }

    /// Builds a runner that launches the deployed node executable
    /// `<root>/.drm/deploy/{node}` with `root` as the working directory.
    /// REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn for_workspace(root: &Path) -> Self {
        let program = root
            .join(".drm")
            .join("deploy")
            .join("{node}")
            .display()
            .to_string();
        Self {
            program,
            args: Vec::new(),
            cwd: Some(root.to_path_buf()),
        }
    }

    /// Substitutes the `{node}` placeholder in a template. REQ: REQ-STUDIO-010.
    fn substitute(template: &str, node: &str) -> String {
        template.replace("{node}", node)
    }
}

impl ProcessRunner for CommandRunner {
    fn spawn(
        &mut self,
        node: &str,
        _config: &RunConfig,
    ) -> Result<Box<dyn NodeHandle>, StudioError> {
        let program = Self::substitute(&self.program, node);
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| Self::substitute(a, node))
            .collect();
        let mut command = Command::new(&program);
        command.args(&args);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| StudioError::Io(format!("spawn `{program}`: {e}")))?;
        let stdout = Arc::new(Mutex::new(String::new()));
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = child.stdout.take() {
            capture(pipe, Arc::clone(&stdout));
        }
        if let Some(pipe) = child.stderr.take() {
            capture(pipe, Arc::clone(&stderr));
        }
        Ok(Box::new(SpawnedNode {
            child,
            stdout,
            stderr,
            killed: false,
        }))
    }
}

/// A real node process handle over a [`std::process::Child`].
/// REQ: REQ-STUDIO-010.
#[derive(Debug)]
struct SpawnedNode {
    child: Child,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    killed: bool,
}

impl NodeHandle for SpawnedNode {
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    fn stdout(&self) -> String {
        self.stdout.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn stderr(&self) -> String {
        self.stderr.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn try_wait(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        }
    }

    fn kill(&mut self) {
        if !self.killed {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.killed = true;
        }
    }
}

/// Reads `reader` line by line into `sink` on a detached thread until EOF.
/// REQ: REQ-STUDIO-010.
fn capture(reader: impl std::io::Read + Send + 'static, sink: Arc<Mutex<String>>) {
    let _ = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buf);
                    if let Ok(mut guard) = sink.lock() {
                        guard.push_str(&text);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{
        CloseChoice, CloseDecision, NodeHandle, ProcessRunner, ProcessStatus, RunConfig,
        SessionStatus, Supervisor,
    };
    use crate::studio::protocol::StudioError;

    static NEXT_PID: AtomicU32 = AtomicU32::new(1000);

    /// Deterministic node handle used by the supervisor tests.
    #[derive(Debug)]
    struct FakeHandle {
        pid: u32,
        out: String,
        running: bool,
    }

    impl NodeHandle for FakeHandle {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }
        fn stdout(&self) -> String {
            self.out.clone()
        }
        fn stderr(&self) -> String {
            String::new()
        }
        fn try_wait(&mut self) -> Option<i32> {
            if self.running { None } else { Some(0) }
        }
        fn kill(&mut self) {
            self.running = false;
        }
    }

    /// Runner that fabricates fake handles (no OS process is spawned).
    #[derive(Debug, Default)]
    struct FakeRunner;

    impl ProcessRunner for FakeRunner {
        fn spawn(
            &mut self,
            node: &str,
            _config: &RunConfig,
        ) -> Result<Box<dyn NodeHandle>, StudioError> {
            let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(FakeHandle {
                pid,
                out: format!("{node} up\n"),
                running: true,
            }))
        }
    }

    /// Builds a run config for `nodes` with the given disconnect policy.
    fn config(nodes: &[&str], leave_running: bool) -> RunConfig {
        RunConfig {
            nodes: nodes.iter().map(|s| (*s).to_string()).collect(),
            leave_running,
        }
    }

    /// REQ: REQ-STUDIO-010 — a session carries id/start time/node processes and
    /// stop terminates them (I-ST10).
    #[test]
    fn supervisor_start_stop_lifecycle() {
        let mut sup = Supervisor::new(Box::new(FakeRunner));
        let id = sup
            .start(config(&["controller"], true))
            .unwrap_or_else(|e| unreachable!("start: {e}"));
        let view = sup.session(&id).unwrap_or_else(|| unreachable!("session"));
        assert_eq!(view.status, SessionStatus::Running);
        assert!(view.started_at > 0, "start time recorded");
        assert_eq!(view.nodes.len(), 1);
        assert!(view.nodes[0].pid.is_some(), "node pid recorded");
        assert!(view.nodes[0].stdout.contains("controller up"));

        sup.stop(&id).unwrap_or_else(|e| unreachable!("stop: {e}"));
        let stopped = sup.session(&id).unwrap_or_else(|| unreachable!("session"));
        assert_eq!(stopped.status, SessionStatus::Stopped);
        assert_eq!(stopped.nodes[0].status, ProcessStatus::Stopped);
    }

    /// REQ: REQ-STUDIO-010 — a disconnect with `leave_running = true` keeps the
    /// session alive; a window close prompts and never kills (I-ST10).
    #[test]
    fn supervisor_outlives_ui() {
        let mut sup = Supervisor::new(Box::new(FakeRunner));
        let id = sup
            .start(config(&["controller"], true))
            .unwrap_or_else(|e| unreachable!("start: {e}"));
        let running = sup.detach();
        assert_eq!(running, vec![id.clone()], "disconnect leaves it running");
        assert_eq!(
            sup.session(&id).unwrap_or_else(|| unreachable!("s")).status,
            SessionStatus::Running
        );
        assert_eq!(
            sup.close_decision(),
            CloseDecision::Prompt {
                sessions: vec![id.clone()]
            }
        );
        let left = sup.apply_close(CloseChoice::LeaveRunning);
        assert_eq!(left, vec![id.clone()]);
        assert_eq!(
            sup.session(&id).unwrap_or_else(|| unreachable!("s")).status,
            SessionStatus::Running
        );
        let stopped = sup.apply_close(CloseChoice::Stop);
        assert!(stopped.is_empty());
        assert_eq!(
            sup.session(&id).unwrap_or_else(|| unreachable!("s")).status,
            SessionStatus::Stopped
        );
    }

    /// REQ: REQ-STUDIO-010 — a disconnect stops sessions whose config forbids
    /// leaving them running.
    #[test]
    fn supervisor_disconnect_stops_when_config_says_so() {
        let mut sup = Supervisor::new(Box::new(FakeRunner));
        let id = sup
            .start(config(&["controller"], false))
            .unwrap_or_else(|e| unreachable!("start: {e}"));
        let running = sup.detach();
        assert!(running.is_empty(), "config says stop on disconnect");
        assert_eq!(
            sup.session(&id).unwrap_or_else(|| unreachable!("s")).status,
            SessionStatus::Stopped
        );
        assert_eq!(sup.close_decision(), CloseDecision::Clear);
    }

    /// REQ: REQ-STUDIO-010 — restart replaces node processes but keeps the id
    /// and config. REQ: REQ-STUDIO-010.
    #[test]
    fn supervisor_restart_replaces_processes() {
        let mut sup = Supervisor::new(Box::new(FakeRunner));
        let id = sup
            .start(config(&["controller"], true))
            .unwrap_or_else(|e| unreachable!("start: {e}"));
        let first_pid = sup.session(&id).unwrap_or_else(|| unreachable!("s")).nodes[0].pid;
        let restarted = sup
            .restart(&id)
            .unwrap_or_else(|e| unreachable!("restart: {e}"));
        assert_eq!(restarted.id, id);
        assert_eq!(restarted.status, SessionStatus::Running);
        assert_ne!(restarted.nodes[0].pid, first_pid, "new process spawned");
    }

    /// REQ: REQ-STUDIO-010 — operating an unknown session is a structured error.
    #[test]
    fn supervisor_unknown_session_is_error() {
        let mut sup = Supervisor::new(Box::new(FakeRunner));
        assert!(matches!(sup.stop("missing"), Err(StudioError::Unknown(_))));
        assert!(matches!(
            sup.restart("missing"),
            Err(StudioError::Unknown(_))
        ));
    }

    /// REQ: REQ-STUDIO-010 — close with no running session is clear.
    #[test]
    fn supervisor_close_clear_when_idle() {
        let sup = Supervisor::new(Box::new(FakeRunner));
        assert_eq!(sup.close_decision(), CloseDecision::Clear);
    }
}
