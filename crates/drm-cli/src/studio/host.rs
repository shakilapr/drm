//! Studio host state machine + stdio loop. The host keeps the
//! `WorkspaceModel` alive across requests, enforces trust and revision guards,
//! and routes every mutation through the `drm-workspace` engine (single
//! semantics, I-ST1). No HTTP server, no ports (`REQ-STUDIO-011`).
//!
//! REQ: REQ-STUDIO-003 (engine-only mutations), REQ-STUDIO-007 (handshake),
//! REQ-STUDIO-009 (revision guard), REQ-STUDIO-011 (stdio host).

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Read as _, Write};
use std::path::PathBuf;

use drm_workspace::model::{ModelError, render_model};
use drm_workspace::verify::verify_lite;

use crate::error::CliError;
use crate::ws::{self, Discovered};

use super::edit;
use super::journal::{ChangeSet, JournalStore};
use super::protocol::{
    Diagnostic, HOST_VERSION, Hello, HelloAck, MAX_CONTROL_BYTES, ModelCommand, PROTOCOL_VERSION,
    StudioError, StudioRequest, StudioResponse, VerifySummary, WorkspaceSnapshot, capabilities,
    parse_bounded, severity_for_check, to_json_line,
};
use super::supervisor::{CommandRunner, ProcessRunner, RunSessionView, Supervisor};

/// Long-lived `drm studio --stdio` host. REQ: REQ-STUDIO-011.
#[derive(Debug)]
pub struct StudioHost {
    root: PathBuf,
    revision: u64,
    trusted: bool,
    opened: bool,
    journals: JournalStore,
    changesets: BTreeMap<String, ChangeSet>,
    supervisor: Supervisor,
}

impl StudioHost {
    /// Creates a host rooted at `root` with the given trust state.
    /// REQ: REQ-STUDIO-011, REQ-STUDIO-012.
    #[must_use]
    pub fn new(root: PathBuf, trusted: bool) -> Self {
        let journals = JournalStore::new(&root);
        let supervisor = Supervisor::new(Box::new(CommandRunner::for_workspace(&root)));
        Self {
            root,
            revision: 0,
            trusted,
            opened: false,
            journals,
            changesets: BTreeMap::new(),
            supervisor,
        }
    }

    /// Creates a host with an injected run-process launcher (tests never spawn
    /// OS processes). REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn with_runner(root: PathBuf, trusted: bool, runner: Box<dyn ProcessRunner>) -> Self {
        let journals = JournalStore::new(&root);
        Self {
            root,
            revision: 0,
            trusted,
            opened: false,
            journals,
            changesets: BTreeMap::new(),
            supervisor: Supervisor::new(runner),
        }
    }

    /// Current workspace revision. REQ: REQ-STUDIO-009.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the workspace is trusted (Build/Run/extensions unlocked).
    /// REQ: REQ-STUDIO-012.
    #[must_use]
    pub fn trusted(&self) -> bool {
        self.trusted
    }

    /// A snapshot of one run session by id. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn run_session(&self, id: &str) -> Option<RunSessionView> {
        self.supervisor.session(id)
    }

    /// Snapshots of every known run session. REQ: REQ-STUDIO-010.
    #[must_use]
    pub fn run_sessions(&self) -> Vec<RunSessionView> {
        self.supervisor
            .session_ids()
            .iter()
            .filter_map(|id| self.supervisor.session(id))
            .collect()
    }

    /// Handles a renderer disconnect: sessions persist per their
    /// `leave_running` policy (I-ST10). Returns the ids still running.
    /// REQ: REQ-STUDIO-010.
    pub fn on_disconnect(&mut self) -> Vec<String> {
        self.supervisor.detach()
    }

    /// Builds the handshake acknowledgement for a client `Hello`.
    /// REQ: REQ-STUDIO-007.
    #[must_use]
    pub fn handshake(&self, _hello: &Hello) -> HelloAck {
        // The host advertises its own version; an older client keeps working and
        // gates on capabilities (never an exact-version requirement).
        HelloAck {
            protocol_version: PROTOCOL_VERSION,
            host_version: HOST_VERSION.to_string(),
            capabilities: capabilities(self.trusted),
        }
    }

    /// Dispatches one typed request through engine calls.
    /// REQ: REQ-STUDIO-003, REQ-STUDIO-007, REQ-STUDIO-009.
    ///
    /// # Errors
    /// A [`StudioError`] for trust, revision, parse, verify, or IO failures;
    /// stale revisions are returned as [`StudioResponse::Conflict`] (not an
    /// error) so the UI can offer Review/Reload/Retry.
    pub fn dispatch(&mut self, req: StudioRequest) -> Result<StudioResponse, StudioError> {
        match req {
            StudioRequest::Hello(hello) => Ok(StudioResponse::HelloAck(self.handshake(&hello))),
            StudioRequest::OpenWorkspace { path } => self.open(&path),
            StudioRequest::Snapshot => {
                let snap = self.snapshot()?;
                Ok(StudioResponse::Snapshot(snap))
            }
            StudioRequest::Verify => {
                let summary = self.verify()?;
                Ok(StudioResponse::Verify(summary))
            }
            StudioRequest::Mutate {
                base_revision,
                command,
            } => self.mutate(base_revision, &command),
            StudioRequest::BuildStart { .. } | StudioRequest::BuildCancel => {
                self.require_trusted()?;
                self.require_open()?;
                Ok(StudioResponse::BuildAccepted {
                    revision: self.revision,
                })
            }
            StudioRequest::RunStart { config } => {
                self.require_trusted()?;
                self.require_open()?;
                let session = self.supervisor.start(config)?;
                Ok(StudioResponse::RunAccepted { session })
            }
            StudioRequest::RunStop { session } => {
                self.require_trusted()?;
                self.supervisor.stop(&session)?;
                Ok(StudioResponse::RunAccepted { session })
            }
            StudioRequest::RunRestart { session } => {
                self.require_trusted()?;
                self.supervisor.restart(&session)?;
                Ok(StudioResponse::RunAccepted { session })
            }
            StudioRequest::Undo { changeset, force } => self.undo(&changeset, force),
            StudioRequest::SubscribeTelemetry { .. } => {
                // The control plane returns only a token; sample bytes travel the
                // binary plane (I-ST3). REQ: REQ-STUDIO-013.
                Ok(StudioResponse::TelemetryToken {
                    token: super::journal::fresh_id(),
                })
            }
        }
    }

    /// Opens a workspace root, recovering journals when trusted.
    /// REQ: REQ-STUDIO-008, REQ-STUDIO-011.
    fn open(&mut self, path: &str) -> Result<StudioResponse, StudioError> {
        self.root = PathBuf::from(path);
        self.journals = JournalStore::new(&self.root);
        if self.trusted {
            self.journals.recover()?;
        }
        self.opened = true;
        self.revision = 1;
        let snap = self.snapshot()?;
        Ok(StudioResponse::Snapshot(snap))
    }

    /// Builds the current revisioned snapshot via the engine.
    /// REQ: REQ-STUDIO-009, REQ-STUDIO-014.
    fn snapshot(&self) -> Result<WorkspaceSnapshot, StudioError> {
        self.require_open()?;
        let discovered = self.discover()?;
        let model = ws::build_model(&discovered).map_err(|e| map_model_errors(&e))?;
        let findings = verify_lite(&model);
        Ok(WorkspaceSnapshot {
            revision: self.revision,
            model: render_model(&model),
            problems: findings.iter().map(diagnostic).collect(),
        })
    }

    /// Runs the engine verifier over the current workspace.
    /// REQ: REQ-STUDIO-003, REQ-STUDIO-014.
    fn verify(&self) -> Result<VerifySummary, StudioError> {
        self.require_open()?;
        let discovered = self.discover()?;
        let model = ws::build_model(&discovered).map_err(|e| map_model_errors(&e))?;
        let findings = verify_lite(&model);
        Ok(VerifySummary {
            green: findings.is_empty(),
            findings: findings
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
        })
    }

    /// Applies a revision-guarded, format-preserving mutation, verifies, and
    /// commits it as a journaled `ChangeSet`. REQ: REQ-STUDIO-005,
    /// REQ-STUDIO-008, REQ-STUDIO-009.
    fn mutate(
        &mut self,
        base_revision: u64,
        command: &ModelCommand,
    ) -> Result<StudioResponse, StudioError> {
        self.require_trusted()?;
        self.require_open()?;
        if base_revision != self.revision {
            return Ok(StudioResponse::Conflict {
                revision: self.revision,
            });
        }
        let discovered = self.discover()?;
        let (node, target, updated) = Self::render_edit(&discovered, command)?;

        // Re-parse + verify before touching the disk (no partial apply).
        let mut modified = discovered.clone();
        for entry in &mut modified.node_inputs {
            if entry.0 == node {
                entry.1 = updated.clone().into_bytes();
            }
        }
        let model = ws::build_model(&modified).map_err(|e| map_model_errors(&e))?;
        let findings = verify_lite(&model);
        if !findings.is_empty() {
            let joined = findings
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(StudioError::Verify(format!("mutation rejected: {joined}")));
        }

        let mut cs = ChangeSet::new(vec![super::journal::output_for(
            &super::journal::fresh_id(),
            target,
            updated,
        )?]);
        self.journals.commit(&mut cs)?;
        let changeset = cs.id.clone();
        self.changesets.insert(cs.id.clone(), cs);
        self.revision += 1;
        Ok(StudioResponse::MutationResult {
            revision: self.revision,
            render: render_model(&model),
            changeset,
        })
    }

    /// Renders `(node, target path, new content)` for a command
    /// (format-preserving). REQ: REQ-STUDIO-005.
    fn render_edit(
        discovered: &Discovered,
        command: &ModelCommand,
    ) -> Result<(String, PathBuf, String), StudioError> {
        match command {
            ModelCommand::NodeUpdate(spec) => {
                let path = discovered.nodes.get(&spec.name).cloned().ok_or_else(|| {
                    StudioError::Unknown(format!("node `{}` is not declared", spec.name))
                })?;
                let before = fs::read_to_string(&path)
                    .map_err(|e| StudioError::Io(format!("read `{}`: {e}", path.display())))?;
                let after = edit::apply_node_update(&before, spec)?;
                Ok((spec.name.clone(), path, after))
            }
            other => Err(StudioError::Unsupported(format!(
                "command `{other:?}` is not implemented in this slice (carry-over M17W4)"
            ))),
        }
    }

    /// Conditional undo: revert only when input hashes still match, unless
    /// `force` (which requires trust). REQ: REQ-STUDIO-002.
    fn undo(&mut self, changeset: &str, force: bool) -> Result<StudioResponse, StudioError> {
        self.require_trusted()?;
        let cs = self
            .changesets
            .get(changeset)
            .cloned()
            .ok_or_else(|| StudioError::Unknown(format!("unknown changeset `{changeset}`")))?;
        if cs.inputs_unchanged() || force {
            self.journals.revert(&cs)?;
            self.changesets.remove(changeset);
            self.revision += 1;
            let snap = self.snapshot()?;
            Ok(StudioResponse::MutationResult {
                revision: self.revision,
                render: snap.model,
                changeset: changeset.to_string(),
            })
        } else {
            Ok(StudioResponse::ConflictExternalChanged {
                changeset: changeset.to_string(),
            })
        }
    }

    /// Discovers the workspace tree (facade owns the walk).
    /// REQ: REQ-STUDIO-003.
    fn discover(&self) -> Result<Discovered, StudioError> {
        ws::discover(&self.root).map_err(map_cli_error)
    }

    /// Rejects operations before a workspace is opened.
    /// REQ: REQ-STUDIO-011.
    fn require_open(&self) -> Result<(), StudioError> {
        if self.opened {
            Ok(())
        } else {
            Err(StudioError::Unknown(
                "no workspace open; send `open_workspace` first".to_string(),
            ))
        }
    }

    /// Rejects execution requests on an untrusted workspace (I-ST11).
    /// REQ: REQ-STUDIO-012.
    fn require_trusted(&self) -> Result<(), StudioError> {
        if self.trusted {
            Ok(())
        } else {
            Err(StudioError::Untrusted(
                "workspace is untrusted; Build/Run/mutations are gated behind [ Trust Workspace ]"
                    .to_string(),
            ))
        }
    }
}

/// Maps a model-build failure to a verify error. REQ: REQ-STUDIO-003.
fn map_model_errors(errs: &[ModelError]) -> StudioError {
    StudioError::Verify(format!(
        "workspace model invalid: {}",
        errs.iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// Maps a facade IO error to a studio error. REQ: REQ-STUDIO-011.
fn map_cli_error(err: CliError) -> StudioError {
    match err {
        CliError::Verify(m) | CliError::Manifest(m) => StudioError::Verify(m),
        CliError::Refused(m) => StudioError::Untrusted(m),
        CliError::Usage(m) | CliError::NotFound(m) => StudioError::Unknown(m),
        CliError::Io(m) | CliError::Lockfile(m) => StudioError::Io(m),
    }
}

/// Renders a verify finding as a wire diagnostic with its severity class.
/// REQ: REQ-STUDIO-014.
fn diagnostic(finding: &drm_workspace::verify::VerifyLiteFinding) -> Diagnostic {
    let check_id = match finding {
        drm_workspace::verify::VerifyLiteFinding::NoPublisher { .. } => {
            drm_workspace::verify::V_SW_NO_PUBLISHER
        }
        drm_workspace::verify::VerifyLiteFinding::MultiplePublishers { .. } => {
            drm_workspace::verify::V_SW_MULTI_PUBLISHER
        }
    };
    Diagnostic {
        severity: severity_for_check(check_id),
        message: finding.to_string(),
    }
}

/// Runs the stdio host: the first request must be `Hello`, then every line is
/// parsed, dispatched through engine calls, and answered. Malformed lines get
/// an error reply and the connection continues. REQ: REQ-STUDIO-007,
/// REQ-STUDIO-011, REQ-STUDIO-013.
///
/// # Errors
/// [`StudioError::Handshake`] when the first request is not `Hello`;
/// [`StudioError::Io`] on a transport failure.
pub fn serve_stdio(
    host: &mut StudioHost,
    rx: &mut impl BufRead,
    tx: &mut impl Write,
) -> Result<(), StudioError> {
    let Some(first) = read_bounded(rx)? else {
        return Ok(());
    };
    let request = parse_bounded(&first)?;
    match request {
        StudioRequest::Hello(hello) => {
            write_line(tx, &StudioResponse::HelloAck(host.handshake(&hello)))?;
        }
        other => {
            let err =
                StudioError::Handshake(format!("first request must be `hello`, got `{other:?}`"));
            write_line(tx, &err.to_response())?;
            return Err(err);
        }
    }

    while let Some(line) = read_bounded(rx)? {
        let response = match parse_bounded(&line) {
            Ok(req) => match host.dispatch(req) {
                Ok(resp) => resp,
                Err(err) => err.to_response(),
            },
            Err(err) => err.to_response(),
        };
        write_line(tx, &response)?;
    }
    Ok(())
}

/// Reads one newline-delimited control frame, bounded by [`MAX_CONTROL_BYTES`].
/// REQ: REQ-STUDIO-013.
///
/// # Errors
/// [`StudioError::Bounds`] when a line exceeds the limit;
/// [`StudioError::Io`] on a transport failure.
fn read_bounded(rx: &mut impl BufRead) -> Result<Option<String>, StudioError> {
    let limit = MAX_CONTROL_BYTES.saturating_add(1);
    let mut buf: Vec<u8> = Vec::new();
    let n = rx
        .by_ref()
        .take(limit as u64)
        .read_until(b'\n', &mut buf)
        .map_err(|e| StudioError::Io(format!("read control line: {e}")))?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_CONTROL_BYTES {
        return Err(StudioError::Bounds(format!(
            "control frame exceeds {MAX_CONTROL_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(buf)
        .map_err(|e| StudioError::Parse(format!("control frame is not UTF-8: {e}")))?;
    Ok(Some(text.trim_end_matches(['\n', '\r']).to_string()))
}

/// Serializes `response` and writes it as one line. REQ: REQ-STUDIO-007.
///
/// # Errors
/// [`StudioError::Io`] on a transport failure.
fn write_line(tx: &mut impl Write, response: &StudioResponse) -> Result<(), StudioError> {
    let line = to_json_line(response)?;
    tx.write_all(line.as_bytes())
        .and_then(|()| tx.write_all(b"\n"))
        .and_then(|()| tx.flush())
        .map_err(|e| StudioError::Io(format!("write response: {e}")))
}
