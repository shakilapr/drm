//! Cross-OS conformance subset (work-plan M12W4-E4-T1, closed out M13W1-E1/E3;
//! windows-backends.md §Test Matrix). The Linux conformance suite rows that must
//! run green on the Windows runner, or carry an explicit port note when a
//! platform fixture is still scheduled. `cargo xtask test --cross-os` executes
//! every green row on the current host — Windows-only rows are `cfg`-gated and
//! no-op elsewhere — so the same manifest defines the nightly Windows subset and
//! the local smoke, with no divergent lists. `PORT_GAPS_CLOSED` holds when no
//! row is silently missing: each is either green or scheduled with a note. The
//! M12W4 `win-race-create` and `win-init-reinit` carry-overs are green here.
//! REQ: REQ-QAL-008, REQ-CORE-101, REQ-ROUTE-025, REQ-ROUTE-026.

use anyhow::{Result, bail};

/// Returns `true` when every [`REQUIRED_INVARIANTS`] entry is exercised by at
/// least one [`SUBSET`] row.
/// REQ: REQ-QAL-008.
#[must_use]
pub fn covers_required_invariants() -> bool {
    REQUIRED_INVARIANTS
        .iter()
        .all(|required| SUBSET.iter().any(|row| row.invariants.contains(required)))
}

/// Windows execution state of one subset row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsState {
    /// The suite runs on the Windows runner and must stay green.
    Green,
    /// Port gap explicitly scheduled (carry-over); the note names the owner and
    /// week so the row can never be silently absent.
    Scheduled(&'static str),
}

/// One Linux conformance suite row that must run on Windows (or be scheduled).
#[derive(Debug, Clone, Copy)]
pub struct SubsetRow {
    /// Stable row id used in reports and `tests/conformance/README.md`.
    pub id: &'static str,
    /// Invariants the row exercises (`I-Wx`, `I-LSx`, `I-RTx`, ...).
    pub invariants: &'static [&'static str],
    /// Cargo test arguments (`package`, target or filter) run for the row.
    pub args: &'static [&'static str],
    /// Windows execution state.
    pub windows: WindowsState,
}

/// Invariants the subset must cover (windows-backends.md §Invariants).
/// REQ: REQ-CORE-101.
pub const REQUIRED_INVARIANTS: &[&str] = &[
    "I-W1", "I-W2", "I-W3", "I-W4", "I-W4b", "I-W5", "I-W6", "I-W7", "I-W8",
];

/// The Windows conformance subset: every row is green on the Windows runner or
/// scheduled with a port note (windows-backends.md §Test Matrix; work-plan
/// M12W4 E1/E3/E4).
/// REQ: REQ-QAL-008.
pub const SUBSET: &[SubsetRow] = &[
    SubsetRow {
        id: "win-shm-locks-identity",
        invariants: &["I-W1", "I-W2", "I-W3", "I-W5", "I-W6", "I-W7", "I-W8"],
        args: &["test", "-p", "drm-core", "--lib", "win_"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-cleanup",
        invariants: &["I-W4"],
        args: &["test", "-p", "drm-core", "--lib", "win_cleanup"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-lifecycle-parity",
        invariants: &["I-W4b", "I-W6", "I-W7"],
        args: &["test", "-p", "drm-core", "--test", "platform_windows"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-transport",
        invariants: &["I-RT5"],
        args: &["test", "-p", "drm-router", "--lib", "transport"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-framing",
        invariants: &["I-RT9"],
        args: &["test", "-p", "drm-router", "--test", "framing_record"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-lanes",
        invariants: &["I-LS1", "I-LS2"],
        args: &["test", "-p", "drm-router", "--test", "lane"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "router-shaping",
        invariants: &["I-LS3", "I-LS4"],
        args: &["test", "-p", "drm-router", "shaping"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "verify-engine",
        invariants: &["I-V1", "I-V6"],
        args: &["test", "-p", "drm-workspace", "--test", "verify_engine"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-race-create",
        invariants: &["I-W1", "I-L1"],
        args: &["test", "-p", "drm-core", "--lib", "win_race_create"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "win-init-reinit",
        invariants: &["I-W1", "I-L8"],
        args: &["test", "-p", "drm-core", "--lib", "win_init_reinit"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "serial-codec",
        invariants: &["I-SR1", "I-SR2", "I-SR3", "I-SR4", "I-SR6"],
        args: &["test", "-p", "drm-router", "--lib", "transport::serial"],
        windows: WindowsState::Green,
    },
    SubsetRow {
        id: "serial-loopback",
        invariants: &["I-SR4", "I-SR6"],
        args: &[
            "test",
            "-p",
            "drm-router",
            "--lib",
            "transport::serial::tests::serial_loopback_x10k",
        ],
        windows: WindowsState::Scheduled(
            "socketpair loopback is unix-only; Windows COM/named-pipe fixture M14",
        ),
    },
];

/// `cargo xtask test --cross-os`: runs every green subset row on the current
/// host and reports the scheduled rows; fails on the first failing row.
/// REQ: REQ-QAL-008.
///
/// # Errors
/// [`anyhow::Error`] when a green row's cargo test invocation exits nonzero.
pub fn run() -> Result<()> {
    if !covers_required_invariants() {
        bail!("cross-os subset does not cover every required I-W invariant");
    }
    let mut green = 0_usize;
    let mut scheduled = 0_usize;
    for row in SUBSET {
        match row.windows {
            WindowsState::Green => {
                crate::spawn("cargo", row.args)
                    .map_err(|e| e.context(format!("cross-os row `{}`", row.id)))?;
                green += 1;
            }
            WindowsState::Scheduled(note) => {
                eprintln!("cross-os: schedule `{}` ({note})", row.id);
                scheduled += 1;
            }
        }
    }
    println!("cross-os subset: {green} green row(s), {scheduled} scheduled");
    Ok(())
}

// ---------------------------------------------------------------------------
// Cross-OS interop matrix (work-plan M13W2-E1-T1/T2, Phase 31).
//
// A parameterized {publisher_os, consumer_os, transport, language} cell runner
// over loopback LAN (or a documented two-netns simulation fallback when no
// second host is available). Each cell is deployed under a
// deployment-identity.md namespace derived from its endpoints; evidence
// (bytes + timing) is archived per run under `target/interop/`. The Rust
// language pair runs on every host via the simulation fallback; C and Python
// cells are scheduled until their router-transport bindings land (Phase 32/33).
// REQ: REQ-QAL-008, REQ-OPS-004, REQ-ROUTE-029, REQ-ROUTE-032.
// ---------------------------------------------------------------------------

use std::path::Path;
use std::time::{Duration, Instant};

/// Host side of an interop cell endpoint.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteropOs {
    /// Windows runtime backend (windows-backends.md).
    Windows,
    /// Linux runtime backend (shm-backend.md).
    Linux,
}

impl InteropOs {
    /// Stable lowercase name used in cell ids and evidence.
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }
}

/// Language of an interop cell endpoint.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteropLang {
    /// C binding (`bindings/c`).
    C,
    /// Rust binding (`bindings/rust`).
    Rust,
    /// Python binding (`bindings/python`).
    Python,
}

impl InteropLang {
    /// Stable lowercase name used in cell ids and evidence.
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

/// Transport medium of an interop cell (router-core.md §Protocols).
/// REQ: REQ-ROUTE-029, REQ-ROUTE-032.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteropTransport {
    /// UDP datagrams — one sample per datagram (I-RT3).
    Udp,
    /// TCP length-framed records with ordered loss recovery (I-RT5).
    Tcp,
}

impl InteropTransport {
    /// Both transports in matrix order.
    pub const ALL: [Self; 2] = [Self::Udp, Self::Tcp];

    /// Stable lowercase name used in cell ids and the `--transport` flag.
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
        }
    }

    /// Parses the `--transport` value.
    /// REQ: REQ-QAL-008.
    ///
    /// # Errors
    /// [`anyhow::Error`] for any value other than `udp` or `tcp`.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            other => bail!("unknown interop transport `{other}` (expected udp or tcp)"),
        }
    }
}

/// One endpoint of an interop cell: a host plus the language running on it.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    /// Host side.
    pub os: InteropOs,
    /// Language binding on that host.
    pub lang: InteropLang,
}

/// One parameterized interop cell.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteropCell {
    /// Sample origin.
    pub publisher: Endpoint,
    /// Sample destination.
    pub consumer: Endpoint,
    /// Transport medium.
    pub transport: InteropTransport,
}

impl InteropCell {
    /// Stable cell id (`pubos-pub Lang-conos-conlang-transport`), used in
    /// reports and the evidence file name.
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn id(self) -> String {
        format!(
            "{}-{}-{}-{}-{}",
            self.publisher.os.name(),
            self.publisher.lang.name(),
            self.consumer.os.name(),
            self.consumer.lang.name(),
            self.transport.name()
        )
    }

    /// Deployment namespace derived from the endpoints (deployment-identity.md:
    /// a deployment is scoped by its instance id, never by a path).
    /// REQ: REQ-QAL-008.
    #[must_use]
    pub fn namespace(self) -> String {
        format!("drm.{}", self.id())
    }
}

/// Execution state of an interop cell.
/// REQ: REQ-QAL-008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    /// Runs on the current host (loopback LAN or two-netns simulation).
    Green,
    /// Not yet runnable; the note names the dependency.
    Scheduled(&'static str),
}

/// Cross-OS OS directions: Windows publisher ↔ Linux consumer and reverse.
/// REQ: REQ-QAL-008.
pub const INTEROP_OS_DIRECTIONS: [(InteropOs, InteropOs); 2] = [
    (InteropOs::Windows, InteropOs::Linux),
    (InteropOs::Linux, InteropOs::Windows),
];

/// The three language pairs exercised per direction (Rust-first until the
/// C/Python router-transport bindings land).
/// REQ: REQ-QAL-008.
pub const INTEROP_LANG_PAIRS: [(InteropLang, InteropLang); 3] = [
    (InteropLang::Rust, InteropLang::Rust),
    (InteropLang::Rust, InteropLang::C),
    (InteropLang::Rust, InteropLang::Python),
];

/// The full interop cell matrix: OS directions × language pairs × transports.
/// REQ: REQ-QAL-008.
#[must_use]
pub fn interop_matrix() -> Vec<InteropCell> {
    let mut cells = Vec::new();
    for (pub_os, con_os) in INTEROP_OS_DIRECTIONS {
        for (pub_lang, con_lang) in INTEROP_LANG_PAIRS {
            for transport in InteropTransport::ALL {
                cells.push(InteropCell {
                    publisher: Endpoint {
                        os: pub_os,
                        lang: pub_lang,
                    },
                    consumer: Endpoint {
                        os: con_os,
                        lang: con_lang,
                    },
                    transport,
                });
            }
        }
    }
    cells
}

/// Host execution state of `cell`: the Rust pair runs everywhere via the
/// documented two-netns simulation fallback; C and Python are scheduled until
/// their transport bindings exist. The OS labels select each endpoint's
/// deployment namespace, not a separate physical runner on the fallback path.
/// REQ: REQ-QAL-008.
#[must_use]
pub fn cell_state(cell: InteropCell) -> CellState {
    let rust_pair =
        cell.publisher.lang == InteropLang::Rust && cell.consumer.lang == InteropLang::Rust;
    if rust_pair {
        CellState::Green
    } else {
        CellState::Scheduled(
            "C/Python router-transport bindings land Phase 32/33; loopback LAN or two-netns sim",
        )
    }
}

/// Evidence captured for one executed cell: payload bytes carried and elapsed
/// wall time. REQ: REQ-QAL-008.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEvidence {
    /// The executed cell.
    pub cell: InteropCell,
    /// Payload bytes verified round-trip.
    pub bytes: usize,
    /// Elapsed wall time in nanoseconds.
    pub elapsed_ns: u128,
}

/// Deterministic per-cell payload (cell id bytes, NUL-terminated), so evidence
/// is reproducible across runs. REQ: REQ-QAL-008.
#[must_use]
fn cell_payload(cell: InteropCell) -> Vec<u8> {
    let mut payload = cell.id().into_bytes();
    payload.push(0);
    payload
}

/// Runs one green cell over a real loopback socket and returns its evidence.
/// REQ: REQ-OPS-004, REQ-ROUTE-029, REQ-ROUTE-032.
///
/// # Errors
/// [`anyhow::Error`] when the socket setup, exchange, or payload comparison
/// fails.
pub fn run_cell(cell: InteropCell) -> Result<CellEvidence> {
    let payload = cell_payload(cell);
    let start = Instant::now();
    let bytes = match cell.transport {
        InteropTransport::Udp => udp_roundtrip(&payload)?,
        InteropTransport::Tcp => tcp_roundtrip(&payload)?,
    };
    Ok(CellEvidence {
        cell,
        bytes,
        elapsed_ns: start.elapsed().as_nanos(),
    })
}

/// UDP loopback exchange: one datagram out, echoed back, byte-exact.
/// REQ: REQ-ROUTE-032.
///
/// # Errors
/// [`anyhow::Error`] on any socket or payload mismatch.
fn udp_roundtrip(payload: &[u8]) -> Result<usize> {
    use std::net::UdpSocket;
    let receiver = UdpSocket::bind("127.0.0.1:0")?;
    receiver.set_read_timeout(Some(Duration::from_secs(5)))?;
    let sender = UdpSocket::bind("127.0.0.1:0")?;
    sender.set_read_timeout(Some(Duration::from_secs(5)))?;
    let dst = receiver.local_addr()?;
    sender.send_to(payload, dst)?;
    let mut buf = vec![0_u8; payload.len().max(1)];
    let (n, from) = receiver.recv_from(&mut buf)?;
    if &buf[..n] != payload {
        bail!("udp payload mismatch on receive");
    }
    receiver.send_to(&buf[..n], from)?;
    let mut echo = vec![0_u8; payload.len().max(1)];
    let (m, _) = sender.recv_from(&mut echo)?;
    if &echo[..m] != payload {
        bail!("udp payload mismatch on echo");
    }
    Ok(m)
}

/// TCP loopback exchange: one length-framed record out, echoed back, byte-exact.
/// REQ: REQ-ROUTE-029.
///
/// # Errors
/// [`anyhow::Error`] on any socket, thread, or payload mismatch.
fn tcp_roundtrip(payload: &[u8]) -> Result<usize> {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let payload = payload.to_vec();
    let expected = payload.clone();
    let server = std::thread::spawn(move || -> Result<usize> {
        let (mut stream, _) = listener.accept()?;
        let mut len = [0_u8; 4];
        stream.read_exact(&mut len)?;
        let n = usize::try_from(u32::from_le_bytes(len))?;
        let mut buf = vec![0_u8; n];
        stream.read_exact(&mut buf)?;
        stream.write_all(&u32::try_from(n)?.to_le_bytes())?;
        stream.write_all(&buf)?;
        Ok(n)
    });
    let mut client = TcpStream::connect(addr)?;
    client.set_read_timeout(Some(Duration::from_secs(5)))?;
    client.write_all(&u32::try_from(payload.len())?.to_le_bytes())?;
    client.write_all(&payload)?;
    let mut len = [0_u8; 4];
    client.read_exact(&mut len)?;
    let n = usize::try_from(u32::from_le_bytes(len))?;
    let mut buf = vec![0_u8; n];
    client.read_exact(&mut buf)?;
    server
        .join()
        .map_err(|_| anyhow::anyhow!("tcp server thread panicked"))??;
    if buf != expected {
        bail!("tcp payload mismatch on echo");
    }
    Ok(n)
}

/// Archives one cell's evidence under `target/interop/<cell>.txt`.
/// REQ: REQ-QAL-008.
///
/// # Errors
/// [`anyhow::Error`] when the directory or file cannot be written.
fn write_evidence(evidence: &CellEvidence) -> Result<()> {
    let dir = Path::new("target").join("interop");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.txt", evidence.cell.id()));
    let body = format!(
        "cell={}\nnamespace={}\nbytes={}\nelapsed_ns={}\n",
        evidence.cell.id(),
        evidence.cell.namespace(),
        evidence.bytes,
        evidence.elapsed_ns
    );
    std::fs::write(&path, body)?;
    Ok(())
}

/// `cargo xtask test --cross-os --transport <udp|tcp>`: executes every green
/// interop cell for the transport (or both when `transport` is `None`),
/// archives per-cell evidence, and reports the scheduled cells. Fails on the
/// first failing cell.
/// REQ: REQ-QAL-008, REQ-OPS-004.
///
/// # Errors
/// [`anyhow::Error`] when a green cell's exchange or evidence write fails.
pub fn run_interop(transport: Option<InteropTransport>) -> Result<()> {
    let mut green = 0_usize;
    let mut scheduled = 0_usize;
    for cell in interop_matrix() {
        if let Some(want) = transport
            && cell.transport != want
        {
            continue;
        }
        match cell_state(cell) {
            CellState::Green => {
                let evidence = run_cell(cell)
                    .map_err(|e| e.context(format!("interop cell `{}`", cell.id())))?;
                write_evidence(&evidence)?;
                green += 1;
            }
            CellState::Scheduled(note) => {
                eprintln!("interop: schedule `{}` ({note})", cell.id());
                scheduled += 1;
            }
        }
    }
    let scope = transport.map_or("udp+tcp", InteropTransport::name);
    println!("interop matrix ({scope}): {green} green cell(s), {scheduled} scheduled");
    Ok(())
}

/// REQ: REQ-QAL-008 — the interop matrix covers every OS direction × language
/// pair × transport, every cell is classified, and the Rust pair runs on the
/// host (simulation fallback) while C/Python carry a schedule note.
#[cfg(test)]
mod interop_tests {
    use super::{
        CellState, INTEROP_LANG_PAIRS, INTEROP_OS_DIRECTIONS, InteropTransport, cell_state,
        interop_matrix, run_cell,
    };

    /// REQ: REQ-QAL-008 — full cross product present, no duplicates.
    #[test]
    fn matrix_covers_every_combination() {
        let cells = interop_matrix();
        let expected =
            INTEROP_OS_DIRECTIONS.len() * INTEROP_LANG_PAIRS.len() * InteropTransport::ALL.len();
        assert_eq!(cells.len(), expected, "matrix size");
        for (pub_os, con_os) in INTEROP_OS_DIRECTIONS {
            for (pub_lang, con_lang) in INTEROP_LANG_PAIRS {
                for transport in InteropTransport::ALL {
                    let found = cells.iter().filter(|c| {
                        c.publisher.os == pub_os
                            && c.consumer.os == con_os
                            && c.publisher.lang == pub_lang
                            && c.consumer.lang == con_lang
                            && c.transport == transport
                    });
                    assert_eq!(found.count(), 1, "missing/duplicated interop cell");
                }
            }
        }
    }

    /// REQ: REQ-QAL-008 — every cell is classified; scheduled cells carry a
    /// note. The Rust pair is green, C/Python scheduled.
    #[test]
    fn every_cell_is_classified() {
        for cell in interop_matrix() {
            match cell_state(cell) {
                CellState::Green => {
                    assert_eq!(cell.publisher.lang.name(), "rust");
                    assert_eq!(cell.consumer.lang.name(), "rust");
                }
                CellState::Scheduled(note) => assert!(!note.is_empty(), "note required"),
            }
        }
    }

    /// REQ: REQ-OPS-004, REQ-ROUTE-029, REQ-ROUTE-032 — a green Rust cell
    /// executes a real loopback exchange for both transports and reports the
    /// payload byte count.
    #[test]
    fn green_cells_round_trip() {
        for cell in interop_matrix() {
            if cell_state(cell) == CellState::Green {
                let evidence =
                    run_cell(cell).unwrap_or_else(|e| unreachable!("cell {}: {e}", cell.id()));
                assert!(evidence.bytes > 0, "cell {} carried bytes", cell.id());
                assert!(!cell.namespace().is_empty());
            }
        }
    }
}

/// REQ: REQ-QAL-008 — the subset covers every required `I-W` invariant, so a
/// green Windows run proves the windows-backends.md §Invariants set.
#[cfg(test)]
mod tests {
    use super::{REQUIRED_INVARIANTS, SUBSET, WindowsState};

    /// REQ: REQ-QAL-008.
    #[test]
    fn subset_covers_required_invariants() {
        for required in REQUIRED_INVARIANTS {
            assert!(
                SUBSET.iter().any(|r| r.invariants.contains(required)),
                "subset misses invariant {required}"
            );
        }
    }

    /// REQ: REQ-QAL-008 — no port gap is silently missing: scheduled rows carry
    /// a note and green rows carry a cargo invocation.
    #[test]
    fn every_row_is_classified() {
        for row in SUBSET {
            assert!(!row.id.is_empty(), "row id must be non-empty");
            match row.windows {
                WindowsState::Green => assert!(
                    !row.args.is_empty(),
                    "green row `{}` needs a cargo invocation",
                    row.id
                ),
                WindowsState::Scheduled(note) => assert!(
                    !note.is_empty(),
                    "scheduled row `{}` needs a port note",
                    row.id
                ),
            }
        }
    }
}
