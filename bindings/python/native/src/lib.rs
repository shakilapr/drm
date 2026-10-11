//! Native `PyO3` core for the DRM Python binding (`bindings/python`, codebase.md
//! 3.4). It wraps the `drm-ffi` C ABI and exposes an **owned-reads-only**
//! surface: [`Consumer::latest`] / [`Consumer::next`] acquire, copy into
//! Python-owned memory, and release the hazard slot before control returns to
//! Python, so no Python-visible object ever references DRM shared memory
//! (bindings-cpp-rust-python.md I-B5/I-B6). The buffer protocol is deliberately
//! not implemented.
//!
//! The pure facade (`drm` package) adds manifest-declared topic resolution and
//! maps undeclared topics to [`ContractError`] (I-B7). `unsafe` is confined to
//! the thin `drm-ffi` call boundary, exactly as in the Rust SDK.
//!
//! REQ: REQ-BIND-014 (undeclared topic ⇒ `ContractError`), REQ-BIND-015 (owned
//! reads only; hazard-free after every call), REQ-BIND-022 (`Send + Sync`
//! classes; per-`Topic` native lock), REQ-BIND-021 (explicit context lifecycle),
//! REQ-BIND-002 (explicit close).

use std::sync::{Mutex, MutexGuard, PoisonError};

use drm_ffi::{
    ENDPOINT_SAMPLE_MAX, ENDPOINT_TOPIC_ID, drm_abi_version, drm_borrow_latest, drm_borrow_release,
    drm_borrow_view, drm_consumer, drm_consumer_close, drm_consumer_open, drm_context,
    drm_context_close, drm_context_open, drm_publish_copy, drm_publisher, drm_publisher_close,
    drm_publisher_open, drm_read_latest, drm_read_next,
};
use drm_types::Status;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

pyo3::create_exception!(
    _native,
    DrmError,
    PyException,
    "Base error for the DRM binding."
);
pyo3::create_exception!(
    _native,
    ContractError,
    DrmError,
    "A topic or operation is not declared in the node manifest (I-B7)."
);
pyo3::create_exception!(
    _native,
    GapError,
    DrmError,
    "A sequential read lapped the cursor; samples were missed (I-C2)."
);

/// Locks a native mutex, recovering the guard from poisoning (a panic in a
/// native call is fail-stop, so poisoning is unreachable in practice; the
/// binding layer still must not `unwrap`).
/// REQ: REQ-BIND-022.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Maps a non-`Ok` DRM status to a Python exception. An undeclared topic is
/// [`ContractError`] (I-B7); everything else is [`DrmError`].
/// REQ: REQ-BIND-014, REQ-BIND-007.
fn status_error(status: Status) -> PyErr {
    match status {
        Status::Permission => ContractError::new_err("topic is not declared in the manifest"),
        other => DrmError::new_err(format!("drm error: {other:?}")),
    }
}

/// Converts a C status code to `Ok(())` or a Python exception, preserving
/// unknown codes verbatim (I-A9). REQ: REQ-BIND-017.
fn check(code: i32) -> PyResult<()> {
    match Status::from_code(code) {
        Some(Status::Ok) => Ok(()),
        Some(status) => Err(status_error(status)),
        None => Err(DrmError::new_err(format!("unknown drm status {code}"))),
    }
}

/// Owned bytes returned by a read; application memory, retainable freely.
/// REQ: REQ-BIND-015.
#[pyclass]
struct Copy {
    /// Copied sample bytes; owned by this Python object, never a shared view.
    data: Vec<u8>,
}

#[pymethods]
impl Copy {
    /// The sample as a Python `bytes` object (a fresh copy).
    /// REQ: REQ-BIND-015.
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// The sample as a `NumPy`-owned `uint8` array. Never a shared-memory view:
    /// `numpy.frombuffer` reads the Python-owned `bytes` and `.copy()` makes the
    /// array independently owned. REQ: REQ-BIND-015.
    fn numpy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let numpy = py.import("numpy")?;
        let owned = PyBytes::new(py, &self.data);
        let view = numpy.call_method1("frombuffer", (owned, "uint8"))?;
        view.call_method0("copy")
    }

    /// Number of bytes in the sample. REQ: REQ-BIND-015.
    fn __len__(&self) -> usize {
        self.data.len()
    }

    /// Alias for [`Copy::bytes`] so `bytes(copy)` works. REQ: REQ-BIND-015.
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// Terse debug representation (length only). REQ: REQ-BIND-015.
    fn __repr__(&self) -> String {
        format!("drm.Copy(len={})", self.data.len())
    }
}

/// Deployment context: owns the in-process backing channel; shared-safe for
/// concurrent endpoint opens (I-A7). Use as a context manager or call
/// [`Context::close`] explicitly. REQ: REQ-BIND-021, REQ-BIND-002.
#[pyclass]
struct Context {
    /// The opaque `drm_context` handle as an address (`0` after close).
    state: Mutex<ContextState>,
}

/// Interior state of a [`Context`]. REQ: REQ-BIND-021.
struct ContextState {
    /// `*mut drm_context` as `usize`; `0` means closed.
    handle: usize,
}

impl Context {
    /// The live handle address, or an error when the context is closed.
    /// REQ: REQ-BIND-021.
    fn live_handle(&self) -> PyResult<usize> {
        let state = lock(&self.state);
        if state.handle == 0 {
            return Err(DrmError::new_err("context is closed"));
        }
        Ok(state.handle)
    }
}

#[pymethods]
impl Context {
    /// Opens a deployment context. `path` is accepted and ignored in this
    /// phase (in-process backing; no manifest/fingerprint load).
    /// REQ: REQ-BIND-021.
    #[staticmethod]
    #[pyo3(signature = (path=None))]
    fn open(path: Option<&str>) -> PyResult<Self> {
        let _ = path; // ignored by the in-process ABI
        let mut raw: *mut drm_context = std::ptr::null_mut();
        // SAFETY: a null path is allowed; `raw` is a valid out-parameter that
        // `drm_context_open` zeroes before any fallible step (I-A2).
        let code = unsafe { drm_context_open(std::ptr::null(), &raw mut raw) };
        check(code)?;
        Ok(Self {
            state: Mutex::new(ContextState {
                handle: raw as usize,
            }),
        })
    }

    /// Closes the context. All derived endpoints must be closed first (I-A7).
    /// REQ: REQ-BIND-002, REQ-BIND-021.
    fn close(&self) -> PyResult<()> {
        let mut state = lock(&self.state);
        if state.handle == 0 {
            return Ok(());
        }
        let handle = state.handle;
        state.handle = 0;
        // SAFETY: `handle` is a live context from `drm_context_open`.
        check(unsafe { drm_context_close(handle as *mut drm_context) })
    }

    /// The topic ids the manifest declares. REQ: REQ-BIND-008.
    #[staticmethod]
    fn declared_topics() -> Vec<u32> {
        vec![ENDPOINT_TOPIC_ID]
    }

    /// Opens a publisher endpoint on `topic_id`; an undeclared topic raises
    /// [`ContractError`] (I-B7). REQ: REQ-BIND-002, REQ-BIND-014.
    fn open_publisher(slf: PyRef<'_, Self>, topic_id: u32) -> PyResult<Publisher> {
        let handle = slf.live_handle()?;
        let mut raw: *mut drm_publisher = std::ptr::null_mut();
        // SAFETY: `handle` is live; `raw` is a valid out-parameter.
        let code =
            unsafe { drm_publisher_open(handle as *mut drm_context, topic_id, &raw mut raw) };
        check(code)?;
        Ok(Publisher {
            ctx: slf.into(),
            state: Mutex::new(EndpointState {
                handle: raw as usize,
            }),
        })
    }

    /// Opens a consumer endpoint on `topic_id`, claiming one borrower record;
    /// an undeclared topic raises [`ContractError`] (I-B7) and an exhausted
    /// borrower lease raises [`DrmError`] (I-B8).
    /// REQ: REQ-BIND-002, REQ-BIND-014, REQ-BIND-010.
    fn open_consumer(slf: PyRef<'_, Self>, topic_id: u32) -> PyResult<Consumer> {
        let handle = slf.live_handle()?;
        let mut raw: *mut drm_consumer = std::ptr::null_mut();
        // SAFETY: `handle` is live; `raw` is a valid out-parameter.
        let code = unsafe { drm_consumer_open(handle as *mut drm_context, topic_id, &raw mut raw) };
        check(code)?;
        Ok(Consumer {
            ctx: slf.into(),
            state: Mutex::new(EndpointState {
                handle: raw as usize,
            }),
        })
    }

    /// Context-manager entry. REQ: REQ-BIND-021.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit; closes the context. REQ: REQ-BIND-002.
    fn __exit__(
        &self,
        _ty: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}

/// Endpoint handle state shared by publishers and consumers.
/// REQ: REQ-BIND-002.
struct EndpointState {
    /// `*mut drm_publisher` / `*mut drm_consumer` as `usize`; `0` after close.
    handle: usize,
}

/// Publisher endpoint. REQ: REQ-BIND-002.
#[pyclass]
struct Publisher {
    /// Keeps the owning context alive for the endpoint's lifetime.
    #[allow(dead_code)]
    ctx: Py<Context>,
    /// Live handle address. REQ: REQ-BIND-002.
    state: Mutex<EndpointState>,
}

#[pymethods]
impl Publisher {
    /// Copy-publishes `data` (zero-padded by the ABI to the fixed sample size).
    /// REQ: REQ-BIND-002, REQ-BIND-008.
    fn publish(&self, data: &[u8]) -> PyResult<()> {
        let state = lock(&self.state);
        if state.handle == 0 {
            return Err(DrmError::new_err("publisher is closed"));
        }
        // SAFETY: `state.handle` is live; `data` is valid for `data.len()` reads.
        let code = unsafe {
            drm_publish_copy(
                state.handle as *mut drm_publisher,
                data.as_ptr(),
                data.len(),
            )
        };
        check(code)
    }

    /// Closes the endpoint. REQ: REQ-BIND-002.
    fn close(&self) -> PyResult<()> {
        let mut state = lock(&self.state);
        if state.handle == 0 {
            return Ok(());
        }
        let handle = state.handle;
        state.handle = 0;
        // SAFETY: `handle` is a live publisher from `drm_publisher_open`.
        check(unsafe { drm_publisher_close(handle as *mut drm_publisher) })
    }

    /// Context-manager entry. REQ: REQ-BIND-002.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit; closes the endpoint. REQ: REQ-BIND-002.
    fn __exit__(
        &self,
        _ty: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}

/// Consumer endpoint (single-borrower, I-A5). Calls on one consumer serialize
/// on the native lock so the cursor stays coherent (I-B10). REQ: REQ-BIND-002,
/// REQ-BIND-006, REQ-BIND-022.
#[pyclass]
struct Consumer {
    /// Keeps the owning context alive for the endpoint's lifetime.
    #[allow(dead_code)]
    ctx: Py<Context>,
    /// Live handle address + cursor lock. REQ: REQ-BIND-022.
    state: Mutex<EndpointState>,
}

#[pymethods]
impl Consumer {
    /// Owned read of the newest committed sample; `None` on an empty channel.
    /// The acquire→copy→release completes before control returns to Python
    /// (I-B5/I-B6). REQ: REQ-BIND-015.
    fn latest(&self) -> PyResult<Option<Copy>> {
        let state = lock(&self.state);
        if state.handle == 0 {
            return Err(DrmError::new_err("consumer is closed"));
        }
        let handle = state.handle as *mut drm_consumer;
        let mut buf = vec![0_u8; ENDPOINT_SAMPLE_MAX];
        let mut out_len: usize = 0;
        // SAFETY: `handle` is live; `buf` is valid for its length; `out_len` is
        // a valid out-parameter. The core never allocates; the binding buffer
        // is copied into owned Python memory before release (I-C9).
        let code =
            unsafe { drm_read_latest(handle, buf.as_mut_ptr(), buf.len(), &raw mut out_len) };
        match Status::from_code(code) {
            Some(Status::Ok) => {
                buf.truncate(out_len);
                Ok(Some(Copy { data: buf }))
            }
            Some(Status::NoSample) => Ok(None),
            Some(status) => Err(status_error(status)),
            None => Err(DrmError::new_err(format!("unknown drm status {code}"))),
        }
    }

    /// Owned sequential read; `None` when caught up, [`GapError`] with the
    /// missed count when the cursor was lapped (I-C2).
    /// REQ: REQ-BIND-015.
    fn next(&self) -> PyResult<Option<Copy>> {
        let state = lock(&self.state);
        if state.handle == 0 {
            return Err(DrmError::new_err("consumer is closed"));
        }
        let handle = state.handle as *mut drm_consumer;
        let mut buf = vec![0_u8; ENDPOINT_SAMPLE_MAX];
        let mut out_len: usize = 0;
        let mut missed: u64 = 0;
        // SAFETY: `handle` is live; `buf`/out-params are valid.
        let code = unsafe {
            drm_read_next(
                handle,
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut out_len,
                &raw mut missed,
            )
        };
        match Status::from_code(code) {
            Some(Status::Ok) => {
                buf.truncate(out_len);
                Ok(Some(Copy { data: buf }))
            }
            Some(Status::NoSample) => Ok(None),
            Some(Status::Gap) => Err(GapError::new_err(missed)),
            Some(status) => Err(status_error(status)),
            None => Err(DrmError::new_err(format!("unknown drm status {code}"))),
        }
    }

    /// Closes the endpoint; an outstanding borrow is a contract violation the
    /// binding never creates (owned reads release before returning).
    /// REQ: REQ-BIND-002.
    fn close(&self) -> PyResult<()> {
        let mut state = lock(&self.state);
        if state.handle == 0 {
            return Ok(());
        }
        let handle = state.handle;
        state.handle = 0;
        // SAFETY: `handle` is a live consumer from `drm_consumer_open`.
        check(unsafe { drm_consumer_close(handle as *mut drm_consumer) })
    }

    /// Diagnostic (I-B5 memory-identity test): address of the backing channel's
    /// committed payload, obtained through a transient borrow that is released
    /// before returning. Not part of the public owned-read API.
    /// REQ: REQ-BIND-015.
    fn _shared_payload_addr(&self) -> PyResult<u64> {
        let state = lock(&self.state);
        if state.handle == 0 {
            return Err(DrmError::new_err("consumer is closed"));
        }
        let handle = state.handle as *mut drm_consumer;
        let mut view = drm_borrow_view {
            data: std::ptr::null(),
            len: 0,
            sequence: 0,
            token: 0,
        };
        // SAFETY: `handle` is live; `view` is a valid out-parameter.
        let code = unsafe { drm_borrow_latest(handle, &raw mut view) };
        check(code)?;
        let addr = view.data as usize;
        // SAFETY: `handle` is live; `token` identifies the outstanding borrow.
        check(unsafe { drm_borrow_release(handle, view.token) })?;
        Ok(addr as u64)
    }

    /// Context-manager entry. REQ: REQ-BIND-002.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit; closes the endpoint. REQ: REQ-BIND-002.
    fn __exit__(
        &self,
        _ty: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        self.close()?;
        Ok(false)
    }
}

/// ABI version probe mirroring `DRM_ABI_VERSION`. REQ: REQ-BIND-018.
#[pyfunction]
fn abi_version() -> u32 {
    drm_abi_version()
}

/// Native extension module `drm._native`. REQ: REQ-BIND-021.
#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Context>()?;
    module.add_class::<Publisher>()?;
    module.add_class::<Consumer>()?;
    module.add_class::<Copy>()?;
    let py = module.py();
    module.add("DrmError", py.get_type::<DrmError>())?;
    module.add("ContractError", py.get_type::<ContractError>())?;
    module.add("GapError", py.get_type::<GapError>())?;
    module.add_function(wrap_pyfunction!(abi_version, module)?)?;
    Ok(())
}
