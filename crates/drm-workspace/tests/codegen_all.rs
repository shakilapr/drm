//! Codegen 4-language integration suite (work-plan M08W3-E2-T1;
//! codegen-invalidation.md §Test Matrix): C/C++/Rust/Python topic modules and
//! node facades render deterministically (I-G1), compile or import, and decode
//! the canonical golden bytes with byte-identical re-encode.
//! REQ: REQ-TOOL-024, REQ-TOOL-026, REQ-TOOL-027, REQ-TOOL-028, REQ-TOOL-029,
//! REQ-TOOL-030, REQ-TOOL-042, REQ-BIND-011, REQ-BIND-012.

mod codegen_all {
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use drm_workspace::codegen::{
        FingerprintDb, GenKind, Lang, execute, existing_generated, out_path, plan,
    };
    use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};

    /// Unique scratch directory under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("drm-gen-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_else(|e| unreachable!("scratch dir: {e}"));
        dir
    }

    /// One-package tree with the given `(stem, source)` topics.
    fn tree(topics: &[(&str, &str)]) -> TreeInput {
        TreeInput {
            packages: vec![PackageInput {
                name: "control".to_string(),
                topics: topics
                    .iter()
                    .map(|(stem, src)| ((*stem).to_string(), src.as_bytes().to_vec()))
                    .collect(),
            }],
            nodes: Vec::new(),
            topology: None,
        }
    }

    /// Builds a model or marks a fixture bug.
    fn model(t: &TreeInput) -> WorkspaceModel {
        build(t).unwrap_or_else(|e| unreachable!("fixture model must build: {e:?}"))
    }

    /// Generates a model into a fresh scratch root and returns the root.
    fn generate(t: &TreeInput, tag: &str) -> PathBuf {
        let m = model(t);
        let root = scratch(tag);
        let mut db = FingerprintDb::default();
        execute(&plan(&m, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("execute: {e:?}"));
        root
    }

    /// Reads every generated file under `root/.drm/gen` into a sorted map.
    fn generated_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let files = existing_generated(root).unwrap_or_else(|e| unreachable!("walk: {e:?}"));
        let mut out = Vec::new();
        for path in files {
            let rel = path
                .strip_prefix(root)
                .map_or_else(|_| path.clone(), std::path::Path::to_path_buf);
            let bytes = fs::read(&path).unwrap_or_else(|e| unreachable!("read: {e}"));
            out.push((rel, bytes));
        }
        out
    }

    /// Whether a tool is present on `PATH`.
    fn tool_found(tool: &str) -> bool {
        Command::new(tool).arg("--version").output().is_ok()
    }

    /// Runs a built executable, returning success.
    fn run_exe(exe: &Path) -> bool {
        Command::new(exe).status().is_ok_and(|s| s.success())
    }

    /// Compiles `driver` with a C-family compiler and returns whether it ran.
    /// `None` means the compiler is absent (the caller skips).
    fn compile_and_run(
        compiler: &str,
        std: &str,
        driver: &Path,
        exe: &Path,
        includes: &[PathBuf],
    ) -> Option<bool> {
        if !tool_found(compiler) {
            return None;
        }
        let mut cmd = Command::new(compiler);
        cmd.arg(std)
            .args(["-Wall", "-Wextra", "-Wpedantic", "-Werror"]);
        for inc in includes {
            cmd.arg("-I").arg(inc);
        }
        cmd.arg(driver).arg("-o").arg(exe);
        let status = cmd.status();
        match status {
            Ok(s) => {
                assert!(s.success(), "{compiler} must compile the generated header");
                Some(run_exe(exe))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => unreachable!("{compiler} spawn: {e}"),
        }
    }

    /// Canonical bytes for `control/target` (`a: u8`, `b: u64`) from the Rust
    /// codec oracle (independent of every generated language path).
    fn fixed_golden(m: &WorkspaceModel) -> Vec<u8> {
        let schema = &m
            .topics
            .get("control/target")
            .unwrap_or_else(|| unreachable!("topic present"))
            .schema;
        let mut buf = [0_u8; 32];
        let n = drm_workspace::canon::value::encode(
            &schema.fields,
            &[
                drm_workspace::canon::value::Value::U8(0x5A),
                drm_workspace::canon::value::Value::U64(0x1122_3344_5566_7788),
            ],
            &mut buf,
        )
        .unwrap_or_else(|e| unreachable!("rust encode: {e:?}"));
        buf[..usize::try_from(n).unwrap_or(0)].to_vec()
    }

    /// `0xAA,0xBB,...` initializer text for a byte slice.
    fn byte_array(bytes: &[u8]) -> String {
        let mut out = String::new();
        for b in bytes {
            let _ = write!(out, "0x{b:02X},");
        }
        out
    }

    /// REQ: REQ-TOOL-029 — every language renders exactly one topic unit and one
    /// node unit, at the stable per-language path (I-G1 layout).
    #[test]
    fn gen_all_languages_present() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let root = scratch("langs");
        let plan = plan(&m, &FingerprintDb::default(), &root);
        for lang in [Lang::C, Lang::Cpp, Lang::Rust, Lang::Python] {
            let out = out_path(&root, lang, GenKind::Topic, "control/target");
            assert!(plan.expected_outputs.contains(&out), "{lang:?} expected");
            assert!(
                plan.dirty_units.iter().any(|u| u.lang == lang),
                "{lang:?} dirty"
            );
        }
        assert_eq!(plan.expected_outputs.len(), 4, "one output per language");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-027, REQ-TOOL-029 — two independent runs render
    /// byte-identical trees across all four languages (I-G1).
    #[test]
    fn gen_all_determinism() {
        let t = tree(&[
            ("target", "a: u8\nb: u64\n"),
            ("state", "x: f32\ns: string[4]\n"),
        ]);
        let m = model(&t);
        let root_a = scratch("all-det-a");
        let root_b = scratch("all-det-b");
        let mut db_a = FingerprintDb::default();
        let mut db_b = FingerprintDb::default();
        execute(&plan(&m, &db_a, &root_a), &mut db_a, &root_a)
            .unwrap_or_else(|e| unreachable!("exec a: {e:?}"));
        execute(&plan(&m, &db_b, &root_b), &mut db_b, &root_b)
            .unwrap_or_else(|e| unreachable!("exec b: {e:?}"));
        assert_eq!(generated_bytes(&root_a), generated_bytes(&root_b));
        let files = generated_bytes(&root_a);
        assert_eq!(files.len(), 8, "2 topics × 4 languages");
        let _ = fs::remove_dir_all(&root_a);
        let _ = fs::remove_dir_all(&root_b);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029, REQ-BIND-011 — the generated C++20 topic
    /// header compiles and round-trips the canonical golden bytes.
    #[test]
    fn gen_cpp_topic_compiles_and_decodes_golden() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let golden = fixed_golden(&m);
        let root = generate(&t, "cpp-fixed");
        let array = byte_array(&golden);
        let driver = format!(
            "#include \"control/target/target.hpp\"\n\
             #include <cstdint>\n\
             int main() {{\n\
             \x20   static const std::uint8_t buf[] = {{{array}}};\n\
             \x20   drm::gen::control_target::ControlTarget t;\n\
             \x20   if (drm::gen::control_target::decode(&t, buf, sizeof buf) != sizeof buf) return 1;\n\
             \x20   if (t.a != 0x5Au) return 2;\n\
             \x20   if (t.b != 0x1122334455667788ull) return 3;\n\
             \x20   std::uint8_t out[sizeof buf];\n\
             \x20   std::size_t n = drm::gen::control_target::encode(t, out, sizeof out);\n\
             \x20   if (n != sizeof buf) return 4;\n\
             \x20   for (std::size_t i = 0; i < sizeof buf; i++) if (out[i] != buf[i]) return 5;\n\
             \x20   return 0;\n\
             }}\n"
        );
        let driver_path = root.join("driver.cpp");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver_cpp");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let includes = vec![
            root.join(".drm/gen/cpp"),
            repo.join("bindings/cpp/include"),
            repo.join("bindings/c/include"),
        ];
        match compile_and_run("g++", "-std=c++20", &driver_path, &exe, &includes) {
            Some(ok) => assert!(ok, "C++ decode/encode parity with the Rust oracle"),
            None => eprintln!("g++ not installed; skipping C++ compile assertion"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029, REQ-BIND-011 — the generated Rust topic
    /// module compiles under `rustc` and round-trips the canonical golden bytes.
    #[test]
    fn gen_rust_topic_compiles_and_decodes_golden() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let golden = fixed_golden(&m);
        let root = generate(&t, "rust-fixed");
        let array = byte_array(&golden);
        let driver = format!(
            "#[path = \"control/target/target.rs\"]\n\
             mod target;\n\
             fn main() {{\n\
             \x20   let buf: [u8; {}] = [{}];\n\
             \x20   let t = target::decode(&buf).expect(\"decode\");\n\
             \x20   assert_eq!(t.a, 0x5A);\n\
             \x20   assert_eq!(t.b, 0x1122334455667788);\n\
             \x20   let mut out = [0u8; {}];\n\
             \x20   let n = target::encode(&t, &mut out).expect(\"encode\");\n\
             \x20   assert_eq!(n, buf.len());\n\
             \x20   assert_eq!(&out[..n], &buf[..]);\n\
             }}\n",
            golden.len(),
            array,
            golden.len()
        );
        let driver_path = root.join(".drm/gen/rust/driver.rs");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        if !tool_found("rustc") {
            eprintln!("rustc not installed; skipping Rust compile assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let exe = root.join("driver_rust");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "bin"])
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap_or_else(|e| unreachable!("rustc spawn: {e}"));
        assert!(status.success(), "rustc must compile the generated module");
        assert!(
            run_exe(&exe),
            "Rust decode/encode parity with the Rust oracle"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029, REQ-BIND-011 — the generated Rust module
    /// for a topic carrying bounded strings/bytes and arrays above 32 bytes must
    /// compile and round-trip. Regression guard for the derived-`Default`
    /// defect: `std` implements `Default` only for arrays of 0..=32 elements, so
    /// a derived impl fails to compile for `[u8; 256]`
    /// (rust-lang/rust#61415; the lifting PR was rejected).
    #[test]
    fn gen_rust_bounded_topic_compiles_and_round_trips() {
        let t = tree(&[(
            "wide",
            "name: string[64]\nblob: bytes[<=256]\npixels: u8[64]\nratio: f64\ncount: u32\n",
        )]);
        let root = generate(&t, "rust-wide");
        let driver = "\
#[path = \"control/wide/target.rs\"]
mod wide;

fn main() {
    let mut v = wide::ControlWide::default();
    let text = b\"wide-topic\";
    v.name_len = text.len() as u32;
    v.name[..text.len()].copy_from_slice(text);
    v.blob_len = 4;
    v.blob[..4].copy_from_slice(&[1, 2, 3, 4]);
    v.pixels[..2].copy_from_slice(&[0xAB, 0xCD]);
    v.ratio = 0.5;
    v.count = 7;

    let mut buf = vec![0u8; wide::SAMPLE_MAX];
    let n = wide::encode(&v, &mut buf).expect(\"encode\");
    let back = wide::decode(&buf[..n]).expect(\"decode\");
    assert_eq!(back, v);
    assert_eq!(back.name_len, 10);
    assert_eq!(back.blob_len, 4);
    assert_eq!(back.pixels[0], 0xAB);
    assert_eq!(back.ratio, 0.5);
    assert_eq!(back.count, 7);
}
";
        let driver_path = root.join(".drm/gen/rust/driver_wide.rs");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        if !tool_found("rustc") {
            eprintln!("rustc not installed; skipping Rust compile assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let exe = root.join("driver_rust_wide");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "bin"])
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap_or_else(|e| unreachable!("rustc spawn: {e}"));
        assert!(
            status.success(),
            "rustc must compile the generated bounded-topic module"
        );
        assert!(run_exe(&exe), "bounded topic encode/decode round-trip");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029, REQ-BIND-011 — the generated Python topic
    /// module imports and round-trips the canonical golden bytes.
    #[test]
    fn gen_python_topic_imports_and_decodes_golden() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let golden = fixed_golden(&m);
        let root = generate(&t, "py-fixed");
        if !tool_found("python3") {
            eprintln!("python3 not installed; skipping Python import assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let array = byte_array(&golden);
        let driver = format!(
            "import sys\n\
             sys.path.insert(0, {gen:?})\n\
             from control.target import target\n\
             buf = bytes([{array}])\n\
             t = target.decode(buf)\n\
             assert t.a == 0x5A, t.a\n\
             assert t.b == 0x1122334455667788, t.b\n\
             out = target.encode(t)\n\
             assert out == buf, (out, buf)\n",
            gen = root.join(".drm/gen/python").to_string_lossy()
        );
        let driver_path = root.join("driver.py");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let status = Command::new("python3")
            .arg(&driver_path)
            .status()
            .unwrap_or_else(|e| unreachable!("python3 spawn: {e}"));
        assert!(
            status.success(),
            "Python decode/encode parity with the Rust oracle"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-042, REQ-BIND-011 — the generated C++ variable
    /// two-region header decodes and re-encodes the golden composite `002`.
    #[test]
    fn gen_cpp_variable_topic_roundtrips() {
        let t = tree(&[("target", "t: u16\ns: string[8]\nv: u32[<=4]\n")]);
        let root = generate(&t, "cpp-var");
        let hex = "9210000003000000140000000200000018000000616263000100000002000000";
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect();
        let array = byte_array(&bytes);
        let driver = format!(
            "#include \"control/target/target.hpp\"\n\
             #include <cstdint>\n\
             #include <cstring>\n\
             int main() {{\n\
             \x20   static const std::uint8_t buf[] = {{{array}}};\n\
             \x20   drm::gen::control_target::ControlTarget t;\n\
             \x20   if (drm::gen::control_target::decode(&t, buf, sizeof buf) != sizeof buf) return 1;\n\
             \x20   if (t.t != 4242u) return 2;\n\
             \x20   if (t.s_len != 3u || std::memcmp(t.s, \"abc\", 3) != 0) return 3;\n\
             \x20   if (t.v_len != 2u || t.v[0] != 1u || t.v[1] != 2u) return 4;\n\
             \x20   std::uint8_t out[drm::gen::control_target::SAMPLE_MAX];\n\
             \x20   if (drm::gen::control_target::encode(t, out, sizeof out) != sizeof buf) return 5;\n\
             \x20   for (std::size_t i = 0; i < sizeof buf; i++) if (out[i] != buf[i]) return 6;\n\
             \x20   return 0;\n\
             }}\n"
        );
        let driver_path = root.join("driver.cpp");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver_cpp_var");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let includes = vec![
            root.join(".drm/gen/cpp"),
            repo.join("bindings/cpp/include"),
            repo.join("bindings/c/include"),
        ];
        match compile_and_run("g++", "-std=c++20", &driver_path, &exe, &includes) {
            Some(ok) => assert!(ok, "C++ variable-topic parity"),
            None => eprintln!("g++ not installed; skipping C++ variable assertion"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-042, REQ-BIND-011 — the generated Rust
    /// variable two-region module round-trips the golden composite `002`.
    #[test]
    fn gen_rust_variable_topic_roundtrips() {
        let t = tree(&[("target", "t: u16\ns: string[8]\nv: u32[<=4]\n")]);
        let root = generate(&t, "rust-var");
        let hex = "9210000003000000140000000200000018000000616263000100000002000000";
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect();
        let array = byte_array(&bytes);
        let driver = format!(
            "#[path = \"control/target/target.rs\"]\n\
             mod target;\n\
             fn main() {{\n\
             \x20   let buf: [u8; {}] = [{}];\n\
             \x20   let t = target::decode(&buf).expect(\"decode\");\n\
             \x20   assert_eq!(t.t, 4242);\n\
             \x20   assert_eq!(t.s_len, 3);\n\
             \x20   assert_eq!(&t.s[..3], b\"abc\");\n\
             \x20   assert_eq!(t.v_len, 2);\n\
             \x20   assert_eq!(&t.v[..2], &[1u32, 2u32]);\n\
             \x20   let mut out = [0u8; target::SAMPLE_MAX];\n\
             \x20   let n = target::encode(&t, &mut out).expect(\"encode\");\n\
             \x20   assert_eq!(&out[..n], &buf[..]);\n\
             }}\n",
            bytes.len(),
            array
        );
        let driver_path = root.join(".drm/gen/rust/driver.rs");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        if !tool_found("rustc") {
            eprintln!("rustc not installed; skipping Rust variable assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let exe = root.join("driver_rust_var");
        let status = Command::new("rustc")
            .args(["--edition", "2024", "--crate-type", "bin"])
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap_or_else(|e| unreachable!("rustc spawn: {e}"));
        assert!(status.success(), "rustc must compile the variable module");
        assert!(run_exe(&exe), "Rust variable-topic parity");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-042, REQ-BIND-011 — the generated Python
    /// variable two-region module round-trips the golden composite `002`.
    #[test]
    fn gen_python_variable_topic_roundtrips() {
        let t = tree(&[("target", "t: u16\ns: string[8]\nv: u32[<=4]\n")]);
        let root = generate(&t, "py-var");
        if !tool_found("python3") {
            eprintln!("python3 not installed; skipping Python variable assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let hex = "9210000003000000140000000200000018000000616263000100000002000000";
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0))
            .collect();
        let array = byte_array(&bytes);
        let driver = format!(
            "import sys\n\
             sys.path.insert(0, {gen:?})\n\
             from control.target import target\n\
             buf = bytes([{array}])\n\
             t = target.decode(buf)\n\
             assert t.t == 4242, t.t\n\
             assert t.s == \"abc\", t.s\n\
             assert t.v == [1, 2], t.v\n\
             assert target.encode(t) == buf\n",
            gen = root.join(".drm/gen/python").to_string_lossy()
        );
        let driver_path = root.join("driver.py");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let status = Command::new("python3")
            .arg(&driver_path)
            .status()
            .unwrap_or_else(|e| unreachable!("python3 spawn: {e}"));
        assert!(status.success(), "Python variable-topic parity");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-028, REQ-BIND-012 — the generated C++ node facade includes
    /// only its declared topics and compiles against the C++ binding.
    #[test]
    fn gen_cpp_node_header_compiles() {
        let mut t = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
        t.nodes.push((
            "controller".to_string(),
            b"name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = [\"control/state\"]\n"
                .to_vec(),
        ));
        let root = generate(&t, "cpp-node");
        let node_header = out_path(&root, Lang::Cpp, GenKind::NodeApi, "controller");
        assert!(node_header.is_file(), "C++ node API unit rendered");
        let driver = "#include \"nodes/controller.hpp\"\nint main() { return 0; }\n";
        let driver_path = root.join("driver.cpp");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver_cpp_node");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let includes = vec![
            root.join(".drm/gen/cpp"),
            repo.join("bindings/cpp/include"),
            repo.join("bindings/c/include"),
        ];
        match compile_and_run("g++", "-std=c++20", &driver_path, &exe, &includes) {
            Some(ok) => assert!(ok, "C++ node header compiles"),
            None => eprintln!("g++ not installed; skipping C++ node assertion"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-028, REQ-BIND-012 — the generated Python node facade imports
    /// and exposes only its declared endpoint operations (node-scoped surface).
    #[test]
    fn gen_python_node_module_imports() {
        let mut t = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
        t.nodes.push((
            "controller".to_string(),
            b"name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = [\"control/state\"]\n"
                .to_vec(),
        ));
        let root = generate(&t, "py-node");
        let node_module = out_path(&root, Lang::Python, GenKind::NodeApi, "controller");
        assert!(node_module.is_file(), "Python node API unit rendered");
        if !tool_found("python3") {
            eprintln!("python3 not installed; skipping Python node assertion");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let driver = format!(
            "import sys\n\
             sys.path.insert(0, {gen:?})\n\
             from nodes import controller\n\
             assert hasattr(controller, \"publish_control_target\"), dir(controller)\n\
             assert hasattr(controller, \"read_control_state\"), dir(controller)\n",
            gen = root.join(".drm/gen/python").to_string_lossy()
        );
        let driver_path = root.join("driver.py");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let status = Command::new("python3")
            .arg(&driver_path)
            .status()
            .unwrap_or_else(|e| unreachable!("python3 spawn: {e}"));
        assert!(status.success(), "Python node facade imports");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-028, REQ-TOOL-065 — the generated Rust node facade renders
    /// one endpoint operation per declared capability and embeds no runtime
    /// handle (node-scoped surface, I-G9).
    #[test]
    fn gen_rust_node_surface() {
        let mut t = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
        t.nodes.push((
            "controller".to_string(),
            b"name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = [\"control/state\"]\n"
                .to_vec(),
        ));
        let root = generate(&t, "rust-node");
        let node_module = out_path(&root, Lang::Rust, GenKind::NodeApi, "controller");
        let text = fs::read_to_string(&node_module)
            .unwrap_or_else(|e| unreachable!("read node module: {e}"));
        assert!(text.contains("pub fn publish_control_target"), "{text}");
        assert!(text.contains("pub fn read_control_state"), "{text}");
        assert!(!text.contains("424242"), "no runtime handle numeral (I-G9)");
        let _ = fs::remove_dir_all(&root);
    }
}
