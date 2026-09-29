//! Linux SHM spike (work-plan M01W1-E1-T3): parent/child round-trip, unlink +
//! re-create, and the EEXIST race probe ×100. Inline POSIX calls only; the real
//! backend bodies land W2–W3 per `docs/algorithms/runtime/shm-backend.md`.
//! REQ: REQ-CORE-072 (exclusive create, exactly one creator), REQ-CORE-071
//! (consumers open existing), REQ-CORE-088 (generated name family).
#![allow(unused)]

fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    {
        match run() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("SPIKE_FAIL: {e}");
                std::process::ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("SPIKE_SKIP: Linux-only spike; Windows variant: windows-backends.md");
        std::process::ExitCode::FAILURE
    }
}

/// REQ: REQ-CORE-072 — spike driver; every probe below is failure-reporting,
/// never panicking (clippy::panic denied).
#[cfg(unix)]
fn run() -> Result<(), String> {
    use std::ffi::CString;

    let name = format!("/drm.spike.{}", std::process::id());
    let c_name = CString::new(name.clone()).map_err(|e| e.to_string())?;

    // Probe 1: round-trip. REQ: REQ-CORE-072.
    let fd = unsafe {
        libc::shm_open(
            c_name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "shm_open(O_CREAT|O_EXCL) errno={}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::ftruncate(fd, 4096) } != 0 {
        return Err(format!(
            "ftruncate errno={}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fd is a valid SHM descriptor of size 4096; mapping covers the whole object.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(format!("mmap errno={}", std::io::Error::last_os_error()));
    }
    unsafe { libc::close(fd) };
    // SAFETY: ptr is a valid 4096-byte shared mapping.
    unsafe { (ptr.cast::<u8>()).write_volatile(42) };

    let child_pid = unsafe { libc::fork() };
    if child_pid < 0 {
        return Err(format!("fork errno={}", std::io::Error::last_os_error()));
    }
    if child_pid == 0 {
        // Child: open-only (never create), read 42, write 43. REQ: REQ-CORE-071.
        let ok = child_roundtrip(&name);
        unsafe { libc::exit(if ok { 0 } else { 1 }) };
    }
    let mut status = 0;
    if unsafe { libc::waitpid(child_pid, &mut status, 0) } != child_pid {
        return Err(format!("waitpid errno={}", std::io::Error::last_os_error()));
    }
    if !libc_ok_status(status) {
        return Err("child round-trip failed".into());
    }
    // SAFETY: ptr is a valid 4096-byte shared mapping.
    let seen = unsafe { ptr.cast::<u8>().read_volatile() };
    if seen != 43 {
        return Err(format!("parent read {seen}, expected 43"));
    }

    // Probe 2: unlink + re-create. REQ: REQ-CORE-072.
    if unsafe { libc::shm_unlink(c_name.as_ptr()) } != 0 {
        return Err(format!(
            "shm_unlink errno={}",
            std::io::Error::last_os_error()
        ));
    }
    let fd2 = unsafe {
        libc::shm_open(
            c_name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd2 < 0 {
        return Err("re-create after unlink failed".into());
    }
    unsafe { libc::close(fd2) };
    if unsafe { libc::shm_unlink(c_name.as_ptr()) } != 0 {
        return Err("final shm_unlink failed".into());
    }
    // SAFETY: unmapping the mapping created above.
    unsafe { libc::munmap(ptr, 4096) };

    // Probe 3: EEXIST race ×100 — per iteration exactly one creator. REQ: REQ-CORE-072.
    for i in 0..100 {
        let race = format!("/drm.spike.race.{i}");
        let c_race = CString::new(race.clone()).map_err(|e| e.to_string())?;
        let mut creators = 0;
        for _ in 0..8 {
            let pid = unsafe { libc::fork() };
            if pid < 0 {
                return Err(format!(
                    "race fork errno={}",
                    std::io::Error::last_os_error()
                ));
            }
            if pid == 0 {
                let created = race_try_create(&race);
                unsafe { libc::exit(if created { 1 } else { 0 }) };
            }
        }
        for _ in 0..8 {
            let mut st = 0;
            if unsafe { libc::waitpid(-1, &mut st, 0) } < 0 {
                return Err(format!(
                    "race waitpid errno={}",
                    std::io::Error::last_os_error()
                ));
            }
            if libc_exit_code(st) == 1 {
                creators += 1;
            }
        }
        if creators != 1 {
            return Err(format!(
                "race iteration {i}: {creators} creators, expected 1"
            ));
        }
        unsafe { libc::shm_unlink(c_race.as_ptr()) };
    }

    println!("SPIKE_OK");
    Ok(())
}

/// REQ: REQ-CORE-071 — child side of the round-trip probe (open-only).
#[cfg(unix)]
fn child_roundtrip(name: &str) -> bool {
    use std::ffi::CString;

    let Ok(c_name) = CString::new(name.to_string()) else {
        return false;
    };
    // SAFETY: c_name is a valid NUL-terminated name; no O_CREAT flag (open-only).
    let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0) };
    if fd < 0 {
        return false;
    }
    // SAFETY: fd is a valid 4096-byte SHM descriptor.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        unsafe { libc::close(fd) };
        return false;
    }
    unsafe { libc::close(fd) };
    // SAFETY: ptr is a valid shared mapping.
    let v = unsafe { ptr.cast::<u8>().read_volatile() };
    let ok = v == 42;
    // SAFETY: ptr is a valid shared mapping.
    unsafe { ptr.cast::<u8>().write_volatile(43) };
    // SAFETY: unmapping the child-local view.
    unsafe { libc::munmap(ptr, 4096) };
    ok
}

/// REQ: REQ-CORE-072 — one race participant; returns whether THIS process created.
#[cfg(unix)]
fn race_try_create(name: &str) -> bool {
    use std::ffi::CString;

    let Ok(c) = CString::new(name.to_string()) else {
        return false;
    };
    // SAFETY: c is a valid NUL-terminated name.
    let fd = unsafe {
        libc::shm_open(
            c.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
            0o600,
        )
    };
    if fd < 0 {
        return false;
    }
    unsafe { libc::close(fd) };
    true
}

/// REQ: REQ-CORE-072 — exit-status decode for waitpid results.
#[cfg(unix)]
fn libc_ok_status(status: i32) -> bool {
    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
}

/// REQ: REQ-CORE-072 — raw exit-code decode for race participants.
#[cfg(unix)]
fn libc_exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    }
}
