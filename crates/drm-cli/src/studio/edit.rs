//! Format-preserving manifest mutation (I-ST6): the engine never rewrites a
//! file it did not author structurally. Node `publishes`/`consumes` edits go
//! through `toml_edit`, so comments, whitespace, and field order survive a GUI
//! mutation (`REQ-STUDIO-005`).
//!
//! REQ: REQ-STUDIO-005 (format-preserving editing), REQ-STUDIO-014 (typed
//! command set).

use toml_edit::{Array, DocumentMut, Item, Value};

use super::protocol::{NodeSpec, StudioError};

/// Applies a [`NodeSpec`] to a node manifest, preserving formatting.
/// REQ: REQ-STUDIO-005.
///
/// # Errors
/// [`StudioError::Parse`] when the manifest is not valid TOML;
/// [`StudioError::Unsupported`] when `publishes`/`consumes` hold a non-array
/// value.
pub fn apply_node_update(manifest: &str, spec: &NodeSpec) -> Result<String, StudioError> {
    let mut doc: DocumentMut = manifest
        .parse()
        .map_err(|e| StudioError::Parse(format!("node manifest is not TOML: {e}")))?;
    set_string_array(&mut doc, "publishes", &spec.publishes)?;
    set_string_array(&mut doc, "consumes", &spec.consumes)?;
    Ok(doc.to_string())
}

/// Renders a fresh node manifest for a create. REQ: REQ-STUDIO-005.
#[must_use]
pub fn render_node_manifest(spec: &NodeSpec) -> String {
    let mut doc = DocumentMut::new();
    doc["name"] = Item::Value(Value::from(spec.name.clone()));
    let mut publishes = Array::new();
    for topic in sorted_unique(&spec.publishes) {
        publishes.push(topic);
    }
    doc["publishes"] = Item::Value(Value::Array(publishes));
    let mut consumes = Array::new();
    for topic in sorted_unique(&spec.consumes) {
        consumes.push(topic);
    }
    doc["consumes"] = Item::Value(Value::Array(consumes));
    doc.to_string()
}

/// Sorts and de-duplicates a topic list deterministically. REQ: REQ-STUDIO-014.
fn sorted_unique(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = items.to_vec();
    out.sort();
    out.dedup();
    out
}

/// Replaces the string array at `key`, keeping the array's formatting decor.
/// REQ: REQ-STUDIO-005.
///
/// # Errors
/// [`StudioError::Unsupported`] when an existing `key` is not an array.
fn set_string_array(
    doc: &mut DocumentMut,
    key: &str,
    values: &[String],
) -> Result<(), StudioError> {
    let values = sorted_unique(values);
    match doc.get_mut(key) {
        Some(Item::Value(Value::Array(arr))) => {
            arr.clear();
            for v in values {
                arr.push(v);
            }
        }
        Some(_) => {
            return Err(StudioError::Unsupported(format!(
                "`{key}` must be an array of topic strings"
            )));
        }
        None => {
            let mut arr = Array::new();
            for v in values {
                arr.push(v);
            }
            doc[key] = Item::Value(Value::Array(arr));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::protocol::NodeSpec;

    /// REQ: REQ-STUDIO-005 — comments, whitespace, and field order survive a
    /// `publishes`/`consumes` mutation (I-ST6).
    #[test]
    fn format_preserving_edit() {
        let before = "\
# controller node
name = \"controller\"

# subscriptions
consumes = [\"control/target\"] # existing
publishes = []
";
        let spec = NodeSpec {
            name: "controller".to_string(),
            publishes: vec!["control/command".to_string()],
            consumes: vec!["control/target".to_string(), "sensor/pose".to_string()],
        };
        let after =
            super::apply_node_update(before, &spec).unwrap_or_else(|e| unreachable!("apply: {e}"));
        assert!(after.contains("# controller node"), "header comment lost");
        assert!(after.contains("# subscriptions"), "section comment lost");
        assert!(after.contains("# existing"), "inline comment lost");
        let name_pos = after.find("name").unwrap_or(usize::MAX);
        let consumes_pos = after.find("consumes").unwrap_or(usize::MAX);
        let publishes_pos = after.find("publishes").unwrap_or(usize::MAX);
        assert!(
            name_pos < consumes_pos && consumes_pos < publishes_pos,
            "order changed"
        );
        assert!(after.contains("sensor/pose"), "new consume missing");
        assert!(after.contains("control/command"), "new publish missing");
    }

    /// REQ: REQ-STUDIO-005 — creating a manifest renders sorted, unique lists.
    #[test]
    fn create_manifest_is_sorted() {
        let spec = NodeSpec {
            name: "camera".to_string(),
            publishes: vec!["b/x".to_string(), "a/y".to_string(), "a/y".to_string()],
            consumes: vec![],
        };
        let text = super::render_node_manifest(&spec);
        assert!(text.contains("name = \"camera\""));
        let a = text.find("a/y").unwrap_or(usize::MAX);
        let b = text.find("b/x").unwrap_or(usize::MAX);
        assert!(a < b, "publishes not sorted: {text}");
    }
}
