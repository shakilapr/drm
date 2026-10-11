//! Codegen C integration suite (work-plan M08W2-E2-T1/T2; codegen-invalidation.md
//! §Test Matrix): determinism (I-G1), affected-only (I-G3), timestamp
//! preservation (I-G2), and golden parity via a compiled C decode.
//! REQ: REQ-TOOL-024, REQ-TOOL-026, REQ-TOOL-027, REQ-TOOL-029, REQ-TOOL-030.

mod codegen_c {
    use std::collections::BTreeSet;
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use drm_workspace::codegen::{
        FingerprintDb, GenKind, Lang, execute, existing_generated, out_path, plan, plan_with,
    };
    use drm_workspace::model::{PackageInput, TreeInput, WorkspaceModel, build};

    /// Unique scratch directory under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("drm-codegen-{tag}-{}-{n}", std::process::id()));
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

    /// REQ: REQ-TOOL-027, REQ-TOOL-029 — two independent runs render
    /// byte-identical trees (I-G1).
    #[test]
    fn gen_determinism() {
        let t = tree(&[
            ("target", "a: u8\nb: u64\n"),
            ("state", "x: f32\ns: string[4]\n"),
        ]);
        let m = model(&t);
        let root_a = scratch("det-a");
        let root_b = scratch("det-b");
        let mut db_a = FingerprintDb::default();
        let mut db_b = FingerprintDb::default();
        let plan_a = plan(&m, &db_a, &root_a);
        let plan_b = plan(&m, &db_b, &root_b);
        execute(&plan_a, &mut db_a, &root_a).unwrap_or_else(|e| unreachable!("exec a: {e:?}"));
        execute(&plan_b, &mut db_b, &root_b).unwrap_or_else(|e| unreachable!("exec b: {e:?}"));
        assert_eq!(generated_bytes(&root_a), generated_bytes(&root_b));
        assert!(!plan_a.dirty_units.is_empty(), "first run renders units");
        let _ = fs::remove_dir_all(&root_a);
        let _ = fs::remove_dir_all(&root_b);
    }

    /// REQ: REQ-TOOL-026, REQ-TOOL-028 — changing one topic regenerates exactly
    /// its unit; unrelated outputs are neither planned nor touched (I-G3).
    #[test]
    fn gen_affected_only() {
        let before = tree(&[("target", "a: u8\nb: u64\n"), ("state", "x: f32\n")]);
        let root = scratch("affected");
        let mut db = FingerprintDb::default();
        let plan_before = plan(&model(&before), &db, &root);
        execute(&plan_before, &mut db, &root).unwrap_or_else(|e| unreachable!("exec: {e:?}"));

        let after = tree(&[("target", "a: u8\nb: u64\nc: u16\n"), ("state", "x: f32\n")]);
        let plan_after = plan(&model(&after), &db, &root);
        let dirty: Vec<&str> = plan_after
            .dirty_units
            .iter()
            .map(|u| u.key.identity.as_str())
            .collect();
        assert_eq!(dirty, vec!["control/target"], "only the changed topic");
        assert_eq!(plan_after.dirty_units[0].kind, GenKind::Topic);
        assert_eq!(plan_after.dirty_units[0].lang, Lang::C);

        execute(&plan_after, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 2: {e:?}"));
        let state = out_path(&root, Lang::C, GenKind::Topic, "control/state");
        assert!(state.is_file(), "unrelated output retained");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-030 — an unchanged fingerprint + existing file leaves the
    /// file (and its mtime) untouched (I-G2).
    #[test]
    fn gen_mtime_preserved() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let root = scratch("mtime");
        let mut db = FingerprintDb::default();
        let plan1 = plan(&m, &db, &root);
        execute(&plan1, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 1: {e:?}"));

        let path = out_path(&root, Lang::C, GenKind::Topic, "control/target");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        set_mtime(&path, old);
        assert_eq!(mtime(&path), old);

        let plan2 = plan(&m, &db, &root);
        assert!(
            plan2.dirty_units.is_empty(),
            "unchanged fingerprint ⇒ no dirty"
        );
        execute(&plan2, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 2: {e:?}"));
        assert_eq!(mtime(&path), old, "file untouched, mtime preserved");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-027, REQ-TOOL-030 — a fingerprint bump whose rendered bytes
    /// are identical still preserves the file and mtime (I-G2).
    #[test]
    fn gen_mtime_preserved_fp_bump() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let root = scratch("fp-bump");
        let mut db = FingerprintDb::default();
        let plan1 = plan_with(&m, &db, &root, "one");
        execute(&plan1, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 1: {e:?}"));

        let path = out_path(&root, Lang::C, GenKind::Topic, "control/target");
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
        set_mtime(&path, old);

        let plan2 = plan_with(&m, &db, &root, "two");
        assert_eq!(
            plan2.dirty_units.len(),
            1,
            "options tag moves the fingerprint"
        );
        execute(&plan2, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 2: {e:?}"));
        assert_eq!(mtime(&path), old, "identical bytes ⇒ no rewrite");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-025 — a removed topic's output is reconciled away (I-G5).
    #[test]
    fn gen_reconciliation() {
        let before = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
        let root = scratch("reconcile");
        let mut db = FingerprintDb::default();
        let plan1 = plan(&model(&before), &db, &root);
        execute(&plan1, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 1: {e:?}"));

        let after = tree(&[("target", "a: u8\n")]);
        let plan2 = plan(&model(&after), &db, &root);
        execute(&plan2, &mut db, &root).unwrap_or_else(|e| unreachable!("exec 2: {e:?}"));
        let state = out_path(&root, Lang::C, GenKind::Topic, "control/state");
        assert!(!state.exists(), "orphan output removed");
        let files = generated_bytes(&root);
        assert_eq!(files.len(), 1, "only the retained topic remains");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029 — generated C compiles under
    /// `-std=c11 -Wall -Wextra -Wpedantic -Werror` and decodes a canonical
    /// sample produced by the Rust codec (golden parity, I-G1).
    #[test]
    fn gen_compiles_and_decodes_golden() {
        let t = tree(&[("target", "a: u8\nb: u64\n")]);
        let m = model(&t);
        let root = scratch("compile");
        let mut db = FingerprintDb::default();
        let plan1 = plan(&m, &db, &root);
        execute(&plan1, &mut db, &root).unwrap_or_else(|e| unreachable!("exec: {e:?}"));

        // Canonical bytes via the Rust codec oracle (independent of the C path).
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

        let mut array = String::new();
        for b in &buf[..usize::try_from(n).unwrap_or(0)] {
            let _ = write!(array, "0x{b:02X},");
        }
        let driver = format!(
            "#include <stddef.h>\n#include <stdint.h>\n#include \"control/target/target.h\"\n\
             int main(void) {{\n\
             \x20   static const uint8_t buf[] = {{{array}}};\n\
             \x20   drm_control_target t;\n\
             \x20   size_t n = drm_control_target_decode(&t, buf, sizeof buf);\n\
             \x20   if (n != sizeof buf) return 1;\n\
             \x20   if (t.a != 0x5Au) return 2;\n\
             \x20   if (t.b != 0x1122334455667788ull) return 3;\n\
             \x20   uint8_t out[sizeof buf];\n\
             \x20   size_t m = drm_control_target_encode(&t, out, sizeof out);\n\
             \x20   if (m != sizeof buf) return 4;\n\
             \x20   {{ size_t i; for (i = 0u; i < sizeof buf; i++) if (out[i] != buf[i]) return 5; }}\n\
             \x20   return 0;\n\
             }}\n"
        );
        let driver_path = root.join("driver.c");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver");
        let include = root.join(".drm/gen/c");
        let status = Command::new("cc")
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(&include)
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status();
        match status {
            Ok(s) => assert!(s.success(), "cc must compile the generated header"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("cc not installed; skipping compile assertion");
                let _ = fs::remove_dir_all(&root);
                return;
            }
            Err(e) => unreachable!("cc spawn: {e}"),
        }
        let run = Command::new(&exe)
            .status()
            .unwrap_or_else(|e| unreachable!("run driver: {e}"));
        assert!(run.success(), "C decode/encode parity with the Rust oracle");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-024, REQ-TOOL-029 — a variable two-region topic header
    /// compiles and decodes the hand-authored golden composite `002`
    /// (`t: u16`, `s: string[8]`, `v: u32[<=4]`; t=4242, s="abc", v=[1,2]).
    #[test]
    fn gen_variable_topic_compiles() {
        let t = tree(&[("target", "t: u16\ns: string[8]\nv: u32[<=4]\n")]);
        let m = model(&t);
        let root = scratch("compile-var");
        let mut db = FingerprintDb::default();
        execute(&plan(&m, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("exec: {e:?}"));

        let hex = "9210000003000000140000000200000018000000616263000100000002000000";
        let mut array = String::new();
        for i in 0..hex.len() / 2 {
            let byte = &hex[2 * i..2 * i + 2];
            let _ = write!(array, "0x{byte},");
        }
        let driver = format!(
            "#include <stddef.h>\n#include <stdint.h>\n#include <string.h>\n\
             #include \"control/target/target.h\"\n\
             int main(void) {{\n\
             \x20   static const uint8_t buf[] = {{{array}}};\n\
             \x20   drm_control_target t;\n\
             \x20   size_t n = drm_control_target_decode(&t, buf, sizeof buf);\n\
             \x20   if (n != sizeof buf) return 1;\n\
             \x20   if (t.t != 4242u) return 2;\n\
             \x20   if (t.s_len != 3u || memcmp(t.s, \"abc\", 3) != 0) return 3;\n\
             \x20   if (t.v_len != 2u || t.v[0] != 1u || t.v[1] != 2u) return 4;\n\
             \x20   uint8_t out[DRM_CONTROL_TARGET_SAMPLE_MAX];\n\
             \x20   size_t m = drm_control_target_encode(&t, out, sizeof out);\n\
             \x20   if (m != sizeof buf) return 5;\n\
             \x20   {{ size_t i; for (i = 0u; i < sizeof buf; i++) if (out[i] != buf[i]) return 6; }}\n\
             \x20   return 0;\n\
             }}\n"
        );
        let driver_path = root.join("driver.c");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver");
        let include = root.join(".drm/gen/c");
        let status = Command::new("cc")
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(&include)
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status();
        match status {
            Ok(s) => assert!(s.success(), "cc must compile the variable header"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("cc not installed; skipping compile assertion");
                let _ = fs::remove_dir_all(&root);
                return;
            }
            Err(e) => unreachable!("cc spawn: {e}"),
        }
        let run = Command::new(&exe)
            .status()
            .unwrap_or_else(|e| unreachable!("run driver: {e}"));
        assert!(run.success(), "variable-topic C decode/encode parity");
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-028, REQ-BIND-012 — a node-scoped C header includes only
    /// its declared topics and compiles against the C ABI.
    #[test]
    fn gen_node_header_compiles() {
        let mut t = tree(&[("target", "a: u8\n"), ("state", "x: f32\n")]);
        t.nodes.push((
            "controller".to_string(),
            b"name = \"controller\"\npublishes = [\"control/target\"]\nconsumes = [\"control/state\"]\n"
                .to_vec(),
        ));
        let m = model(&t);
        let root = scratch("compile-node");
        let mut db = FingerprintDb::default();
        execute(&plan(&m, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("exec: {e:?}"));

        let node_header = out_path(&root, Lang::C, GenKind::NodeApi, "controller");
        assert!(node_header.is_file(), "node API unit rendered");
        let driver = "#include \"nodes/controller.h\"\nint main(void) { return 0; }\n";
        let driver_path = root.join("driver.c");
        fs::write(&driver_path, driver).unwrap_or_else(|e| unreachable!("write driver: {e}"));
        let exe = root.join("driver");
        let include = root.join(".drm/gen/c");
        let abi = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings/c/include");
        let status = Command::new("cc")
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(&include)
            .arg("-I")
            .arg(abi)
            .arg(&driver_path)
            .arg("-o")
            .arg(&exe)
            .status();
        match status {
            Ok(s) => assert!(s.success(), "cc must compile the node header"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("cc not installed; skipping compile assertion");
                let _ = fs::remove_dir_all(&root);
                return;
            }
            Err(e) => unreachable!("cc spawn: {e}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// REQ: REQ-TOOL-026 — `expected_outputs` is the complete tree even when
    /// nothing is dirty (I-G3: reconciliation uses the expected set).
    #[test]
    fn gen_expected_vs_dirty() {
        let t = tree(&[("target", "a: u8\n")]);
        let m = model(&t);
        let root = scratch("expected");
        let mut db = FingerprintDb::default();
        execute(&plan(&m, &db, &root), &mut db, &root)
            .unwrap_or_else(|e| unreachable!("exec 1: {e:?}"));
        let plan2 = plan(&m, &db, &root);
        assert!(plan2.dirty_units.is_empty());
        let expected: BTreeSet<PathBuf> = plan2.expected_outputs.clone();
        assert_eq!(expected.len(), 1, "expected tree is complete");
        assert!(expected.iter().all(|p| p.is_file()));
        let _ = fs::remove_dir_all(&root);
    }

    /// Sets the modification time of `path` (test fixture only).
    fn set_mtime(path: &Path, when: SystemTime) {
        let file = fs::File::options()
            .write(true)
            .open(path)
            .unwrap_or_else(|e| unreachable!("open for times: {e}"));
        file.set_times(fs::FileTimes::new().set_modified(when))
            .unwrap_or_else(|e| unreachable!("set_times: {e}"));
    }

    /// Reads the modification time of `path`.
    fn mtime(path: &Path) -> SystemTime {
        fs::metadata(path)
            .unwrap_or_else(|e| unreachable!("metadata: {e}"))
            .modified()
            .unwrap_or_else(|e| unreachable!("modified: {e}"))
    }
}
