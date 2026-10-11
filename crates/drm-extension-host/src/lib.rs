//! WASM extension host
//! ([studio-protocol.md](../../docs/algorithms/studio/studio-protocol.md)
//! §Invariants `I-ST5`; [ui-architecture.md](../../docs/ui-architecture.md)
//! §Extension Platform). Extensions execute in a `wasmtime` sandbox under a
//! fixed [`ExtensionBudget`]: a fuel budget bounds CPU and a [`StoreLimits`]
//! memory cap bounds allocation. A runaway extension — an unbounded loop or an
//! oversize allocation — is stopped deterministically, and the host engine is
//! left reusable, so one bad extension can never take down the host or the
//! engine. REQ: REQ-STUDIO-004, REQ-STUDIO-020, REQ-STUDIO-021, REQ-STUDIO-022.

#![forbid(unsafe_code)]

use thiserror::Error;
use wasmtime::{
    Caller, Config, Engine, Func, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
};

/// The guest entry point every extension module exports. REQ: REQ-STUDIO-004.
pub const ENTRY_POINT: &str = "drm_ext_run";

/// The optional guest teardown export, called during deterministic teardown.
/// REQ: REQ-STUDIO-021.
pub const TEARDOWN_POINT: &str = "drm_ext_teardown";

/// The host import module that backs the `studio-extension-api` surface.
/// REQ: REQ-STUDIO-021.
pub const HOST_API_MODULE: &str = "drm_studio_api";

/// The single host call on the `studio-extension-api` surface: the guest sends
/// an `i32` request and receives an `i32` reply.
/// REQ: REQ-STUDIO-021.
pub const HOST_API_CALL: &str = "call";

/// Default fuel budget for a single extension call.
/// REQ: REQ-STUDIO-004.
pub const DEFAULT_FUEL: u64 = 10_000_000;

/// Default memory cap for a single extension call: 16 MiB.
/// REQ: REQ-STUDIO-004.
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 16 * 1024 * 1024;

/// The preemption strategy recorded by the M22W1 SPIKE. Fuel metering is chosen
/// over wall-clock epoch interruption: fuel is deterministic (a fixed budget
/// stops at the same guest instruction on every host), needs no background
/// ticker thread, and its overhead is bounded and measurable.
/// REQ: REQ-STUDIO-020.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreemptionStrategy {
    /// Deterministic fuel metering: a fixed instruction budget stops the guest
    /// at the same point on every host, with no background thread. Chosen.
    Fuel,
    /// Wall-clock epoch interruption: a ticker thread bumps an epoch; the stop
    /// point depends on host scheduling, so termination is not reproducible.
    Epoch,
}

impl PreemptionStrategy {
    /// The strategy chosen by the M22W1 SPIKE (`Fuel`), recorded in
    /// [studio-protocol.md](../../docs/algorithms/studio/studio-protocol.md).
    /// REQ: REQ-STUDIO-020.
    pub const CHOSEN: Self = Self::Fuel;
}

/// The recovery strategy recorded by the M22W4 chaos EXP: after a mid-call
/// stop, the supervisor restarts the extension rather than failing closed,
/// because restart restores service deterministically while fail-closed loses
/// the extension until a human re-enables it.
/// REQ: REQ-STUDIO-022.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStrategy {
    /// Open a fresh session after a stop (chosen).
    SupervisorRestart,
    /// Tear the extension down and refuse further calls (rejected).
    FailClosed,
}

impl RecoveryStrategy {
    /// The strategy chosen by the M22W4 chaos EXP (`SupervisorRestart`).
    /// REQ: REQ-STUDIO-022.
    pub const CHOSEN: Self = Self::SupervisorRestart;
}

/// A fixed extension resource budget: a fuel (CPU/instruction) cap and a memory
/// cap. Both are declared before the extension runs and never grow (`I-ST5`).
/// REQ: REQ-STUDIO-004.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionBudget {
    fuel: u64,
    max_memory_bytes: usize,
}

impl ExtensionBudget {
    /// A budget with `fuel` instructions and `max_memory_bytes` of linear
    /// memory. REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn new(fuel: u64, max_memory_bytes: usize) -> Self {
        Self {
            fuel,
            max_memory_bytes,
        }
    }

    /// The declared fuel budget. REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn fuel(self) -> u64 {
        self.fuel
    }

    /// The declared memory cap in bytes. REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn max_memory_bytes(self) -> usize {
        self.max_memory_bytes
    }
}

impl Default for ExtensionBudget {
    /// The default budget ([`DEFAULT_FUEL`], [`DEFAULT_MAX_MEMORY_BYTES`]).
    /// REQ: REQ-STUDIO-004.
    fn default() -> Self {
        Self::new(DEFAULT_FUEL, DEFAULT_MAX_MEMORY_BYTES)
    }
}

/// Why an extension stopped. Every variant is a deterministic, engine-safe stop;
/// none aborts or corrupts the host (`I-ST5`).
/// REQ: REQ-STUDIO-004, REQ-STUDIO-020.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtensionError {
    /// The fuel budget was exhausted; the guest was stopped deterministically
    /// (`I-ST5`). REQ: REQ-STUDIO-020.
    #[error("extension fuel exhausted (deterministic stop)")]
    FuelExhausted,
    /// The guest tried to allocate beyond the declared memory cap (`I-ST5`).
    /// REQ: REQ-STUDIO-004.
    #[error("extension exceeded its {limit}-byte memory cap")]
    MemoryLimit {
        /// The declared cap that the guest exceeded, in bytes.
        limit: usize,
    },
    /// The module could not be compiled. REQ: REQ-STUDIO-004.
    #[error("extension compile failed: {0}")]
    Compile(String),
    /// The module could not be instantiated or its entry point was missing.
    /// REQ: REQ-STUDIO-004.
    #[error("extension instantiation failed: {0}")]
    Instantiate(String),
    /// The guest trapped for a reason other than fuel or memory (`I-ST5`).
    /// REQ: REQ-STUDIO-004.
    #[error("extension trapped: {0}")]
    Trap(String),
}

/// The observable result of one extension invocation (a `call` or a
/// `teardown`): fuel consumed and host-API calls made.
/// REQ: REQ-STUDIO-004, REQ-STUDIO-021.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallOutcome {
    fuel_consumed: u64,
    api_calls: u32,
}

impl CallOutcome {
    /// Fuel consumed by this invocation (metering metric, `I-ST5`).
    /// REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn fuel_consumed(self) -> u64 {
        self.fuel_consumed
    }

    /// Host `studio-extension-api` calls made during this invocation.
    /// REQ: REQ-STUDIO-021.
    #[must_use]
    pub const fn api_calls(self) -> u32 {
        self.api_calls
    }
}

/// The aggregate result of a full extension lifecycle (`load` → `call` →
/// `teardown`). REQ: REQ-STUDIO-021.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionRun {
    fuel_consumed: u64,
    api_calls: u32,
}

impl ExtensionRun {
    /// Total fuel consumed across the lifecycle. REQ: REQ-STUDIO-021.
    #[must_use]
    pub const fn fuel_consumed(self) -> u64 {
        self.fuel_consumed
    }

    /// Total host-API calls made across the lifecycle. REQ: REQ-STUDIO-021.
    #[must_use]
    pub const fn api_calls(self) -> u32 {
        self.api_calls
    }
}

/// The host API an extension may call. The guest reaches it through the
/// [`HOST_API_MODULE`]/[`HOST_API_CALL`] import; the concrete implementation is
/// supplied per session. This mirrors the `packages/studio-extension-api`
/// surface. REQ: REQ-STUDIO-021.
pub trait ExtensionApi {
    /// Handles one guest request and returns the reply.
    /// REQ: REQ-STUDIO-021.
    fn call(&mut self, request: i32) -> i32;
}

/// An [`ExtensionApi`] that answers every request with `0`; used by budget
/// fixtures that exercise preemption only. REQ: REQ-STUDIO-021.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopApi;

impl ExtensionApi for NoopApi {
    /// Returns `0` for every request. REQ: REQ-STUDIO-021.
    fn call(&mut self, _request: i32) -> i32 {
        0
    }
}

/// Per-store state: the memory limiter, the host API, and the call counter
/// (`I-ST5`). REQ: REQ-STUDIO-004.
struct HostState {
    limits: StoreLimits,
    api: Box<dyn ExtensionApi>,
    api_calls: u32,
}

/// A live extension session: one instantiated module with its store. Dropping
/// the session is deterministic teardown; [`ExtensionSession::teardown`] runs
/// the optional guest teardown export first and reports the metrics.
/// REQ: REQ-STUDIO-021.
pub struct ExtensionSession {
    store: Store<HostState>,
    entry: Func,
    teardown: Option<Func>,
    budget: ExtensionBudget,
}

impl ExtensionSession {
    /// The declared budget enforced for this session. REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn budget(&self) -> ExtensionBudget {
        self.budget
    }

    /// Runs the extension entry point once under the budget. Returns the
    /// invocation metrics, or a deterministic stop.
    ///
    /// # Errors
    /// [`ExtensionError::FuelExhausted`] on fuel exhaustion,
    /// [`ExtensionError::MemoryLimit`] on a memory-cap breach, or
    /// [`ExtensionError::Trap`] for any other guest trap.
    /// REQ: REQ-STUDIO-004, REQ-STUDIO-021.
    pub fn call(&mut self) -> Result<CallOutcome, ExtensionError> {
        let before = self.store.data().api_calls;
        self.store
            .set_fuel(self.budget.fuel)
            .map_err(|e| ExtensionError::Instantiate(e.to_string()))?;
        let entry = self.entry;
        entry
            .call(&mut self.store, &[], &mut [])
            .map_err(|e| classify(&e, self.budget))?;
        Ok(self.outcome(before))
    }

    /// Runs the optional teardown export and consumes the session. The stop is
    /// deterministic: a missing teardown export is a no-op.
    ///
    /// # Errors
    /// As [`ExtensionSession::call`].
    /// REQ: REQ-STUDIO-021.
    pub fn teardown(mut self) -> Result<CallOutcome, ExtensionError> {
        let before = self.store.data().api_calls;
        self.store
            .set_fuel(self.budget.fuel)
            .map_err(|e| ExtensionError::Instantiate(e.to_string()))?;
        if let Some(teardown) = self.teardown {
            teardown
                .call(&mut self.store, &[], &mut [])
                .map_err(|e| classify(&e, self.budget))?;
        }
        Ok(self.outcome(before))
    }

    /// Builds a [`CallOutcome`] from the call counter delta and remaining fuel.
    /// REQ: REQ-STUDIO-004.
    fn outcome(&self, api_calls_before: u32) -> CallOutcome {
        let remaining = self.store.get_fuel().unwrap_or(0);
        let delta = self.store.data().api_calls.saturating_sub(api_calls_before);
        CallOutcome {
            fuel_consumed: self.budget.fuel.saturating_sub(remaining),
            api_calls: delta,
        }
    }
}

/// The extension host: one `wasmtime` [`Engine`] configured for fuel metering,
/// reused across extensions and calls. A stopped extension leaves the engine
/// reusable — the engine is unaffected (`I-ST5`).
/// REQ: REQ-STUDIO-004.
#[derive(Debug)]
pub struct ExtensionHost {
    engine: Engine,
    budget: ExtensionBudget,
}

impl ExtensionHost {
    /// Builds a host engine with fuel metering enabled and the declared budget.
    ///
    /// # Errors
    /// [`ExtensionError::Instantiate`] if the `wasmtime` engine cannot be
    /// created. REQ: REQ-STUDIO-004, REQ-STUDIO-020.
    pub fn new(budget: ExtensionBudget) -> Result<Self, ExtensionError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        Engine::new(&config)
            .map(|engine| Self { engine, budget })
            .map_err(|e| ExtensionError::Instantiate(e.to_string()))
    }

    /// The declared budget this host enforces. REQ: REQ-STUDIO-004.
    #[must_use]
    pub const fn budget(&self) -> ExtensionBudget {
        self.budget
    }

    /// Compiles `wasm` bytes (WAT text or a binary module) into a reusable
    /// [`Module`].
    ///
    /// # Errors
    /// [`ExtensionError::Compile`] when the bytes are not a valid module.
    /// REQ: REQ-STUDIO-004.
    pub fn load(&self, wasm: &[u8]) -> Result<Module, ExtensionError> {
        Module::new(&self.engine, wasm).map_err(|e| ExtensionError::Compile(e.to_string()))
    }

    /// Loads `wasm` and opens a session bound to `api` (`load`).
    ///
    /// # Errors
    /// [`ExtensionError::Compile`] on invalid bytes, or
    /// [`ExtensionError::Instantiate`]/[`ExtensionError::MemoryLimit`] if the
    /// module cannot be instantiated under the budget.
    /// REQ: REQ-STUDIO-004, REQ-STUDIO-021.
    pub fn open(
        &self,
        wasm: &[u8],
        api: Box<dyn ExtensionApi>,
    ) -> Result<ExtensionSession, ExtensionError> {
        let module = self.load(wasm)?;
        self.open_module(&module, api)
    }

    /// Opens a session for an already-loaded [`Module`].
    ///
    /// # Errors
    /// As [`ExtensionHost::open`] minus the compile step.
    /// REQ: REQ-STUDIO-004, REQ-STUDIO-021.
    pub fn open_module(
        &self,
        module: &Module,
        api: Box<dyn ExtensionApi>,
    ) -> Result<ExtensionSession, ExtensionError> {
        let mut store = Store::new(
            &self.engine,
            HostState {
                limits: StoreLimitsBuilder::new()
                    .memory_size(self.budget.max_memory_bytes)
                    .build(),
                api,
                api_calls: 0,
            },
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.budget.fuel)
            .map_err(|e| ExtensionError::Instantiate(e.to_string()))?;

        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap(
                HOST_API_MODULE,
                HOST_API_CALL,
                |mut caller: Caller<'_, HostState>, request: i32| -> i32 {
                    let state = caller.data_mut();
                    state.api_calls = state.api_calls.saturating_add(1);
                    state.api.call(request)
                },
            )
            .map_err(|e| ExtensionError::Instantiate(e.to_string()))?;

        let instance = linker
            .instantiate(&mut store, module)
            .map_err(|e| classify(&e, self.budget))?;
        let entry = instance.get_func(&mut store, ENTRY_POINT).ok_or_else(|| {
            ExtensionError::Instantiate(format!("missing export `{ENTRY_POINT}`"))
        })?;
        let teardown = instance.get_func(&mut store, TEARDOWN_POINT);
        Ok(ExtensionSession {
            store,
            entry,
            teardown,
            budget: self.budget,
        })
    }

    /// Runs a full lifecycle in one call: `load` → `call` → `teardown`.
    ///
    /// # Errors
    /// As [`ExtensionHost::open`]/[`ExtensionSession::call`].
    /// REQ: REQ-STUDIO-004, REQ-STUDIO-021.
    pub fn run(&self, wasm: &[u8]) -> Result<ExtensionRun, ExtensionError> {
        self.run_with_api(wasm, Box::new(NoopApi))
    }

    /// Runs a full lifecycle with a supplied host API.
    ///
    /// # Errors
    /// As [`ExtensionHost::open`]/[`ExtensionSession::call`].
    /// REQ: REQ-STUDIO-021.
    pub fn run_with_api(
        &self,
        wasm: &[u8],
        api: Box<dyn ExtensionApi>,
    ) -> Result<ExtensionRun, ExtensionError> {
        let mut session = self.open(wasm, api)?;
        let call = session.call()?;
        let teardown = session.teardown()?;
        Ok(ExtensionRun {
            fuel_consumed: call.fuel_consumed.saturating_add(teardown.fuel_consumed),
            api_calls: call.api_calls.saturating_add(teardown.api_calls),
        })
    }
}

/// Classifies a `wasmtime` error: out-of-fuel ⇒ deterministic fuel stop, a
/// memory-limits error ⇒ memory cap, otherwise a generic trap.
/// REQ: REQ-STUDIO-004, REQ-STUDIO-020.
fn classify(err: &wasmtime::Error, budget: ExtensionBudget) -> ExtensionError {
    if let Some(trap) = err.downcast_ref::<Trap>()
        && *trap == Trap::OutOfFuel
    {
        return ExtensionError::FuelExhausted;
    }
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("memory") && (lower.contains("limit") || lower.contains("exceed")) {
        return ExtensionError::MemoryLimit {
            limit: budget.max_memory_bytes,
        };
    }
    ExtensionError::Trap(msg)
}
