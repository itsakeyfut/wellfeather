use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use slint::ComponentHandle;

// ---------------------------------------------------------------------------
// Internal tree representation
// ---------------------------------------------------------------------------

struct InternalNode {
    key: String,
    path: String,
    kind: NodeKind,
}

enum NodeKind {
    JsonObject(Vec<InternalNode>),
    JsonArray(Vec<InternalNode>),
    JsonString(String),
    JsonNumber(String),
    JsonBool(bool),
    JsonNull,
    XmlElement(Vec<InternalNode>),
    XmlAttr(String),
    XmlText(String),
}

struct TreeViewerState {
    roots: Vec<InternalNode>,
    collapsed: HashSet<String>,
    mode: i32, // 1=json, 2=xml
}

// ---------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------

fn json_value_to_node(v: &serde_json::Value, key: String, path: String) -> InternalNode {
    match v {
        serde_json::Value::Object(map) => {
            let children = map
                .iter()
                .map(|(k, child)| {
                    let child_path = format!("{}.{}", path, k);
                    json_value_to_node(child, k.clone(), child_path)
                })
                .collect();
            InternalNode {
                key,
                path,
                kind: NodeKind::JsonObject(children),
            }
        }
        serde_json::Value::Array(arr) => {
            let children = arr
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let child_key = format!("[{i}]");
                    let child_path = format!("{path}[{i}]");
                    json_value_to_node(child, child_key, child_path)
                })
                .collect();
            InternalNode {
                key,
                path,
                kind: NodeKind::JsonArray(children),
            }
        }
        serde_json::Value::String(s) => InternalNode {
            key,
            path,
            kind: NodeKind::JsonString(s.clone()),
        },
        serde_json::Value::Number(n) => InternalNode {
            key,
            path,
            kind: NodeKind::JsonNumber(n.to_string()),
        },
        serde_json::Value::Bool(b) => InternalNode {
            key,
            path,
            kind: NodeKind::JsonBool(*b),
        },
        serde_json::Value::Null => InternalNode {
            key,
            path,
            kind: NodeKind::JsonNull,
        },
    }
}

fn parse_json(text: &str) -> Option<TreeViewerState> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    // Only treat top-level objects/arrays as tree-viewable; primitives fall back to text.
    match &v {
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {}
        _ => return None,
    }
    let root = json_value_to_node(&v, String::new(), String::new());
    Some(TreeViewerState {
        roots: vec![root],
        collapsed: HashSet::new(),
        mode: 1,
    })
}

// ---------------------------------------------------------------------------
// XML parsing
// ---------------------------------------------------------------------------

fn parse_xml(text: &str) -> Option<TreeViewerState> {
    use quick_xml::Reader;
    use quick_xml::events::Event as XmlEvent;

    let mut reader = Reader::from_str(text.trim());
    reader.config_mut().trim_text(true);

    // Stack: (tag_name, path, accumulated_children)
    let mut stack: Vec<(String, String, Vec<InternalNode>)> = Vec::new();
    // Counts siblings of the same tag name under a given parent path
    let mut sibling_counts: std::collections::HashMap<String, usize> = Default::default();

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(e)) => {
                let tag = std::str::from_utf8(e.name().as_ref()).ok()?.to_string();
                let parent_path = stack.last().map(|(_, p, _)| p.as_str()).unwrap_or("");
                let key = format!("{}/{}", parent_path, tag);
                let idx = *sibling_counts.entry(key.clone()).or_insert(0);
                sibling_counts.insert(key.clone(), idx + 1);
                let path = format!("{}[{}]", key, idx);

                let mut attr_nodes = Vec::new();
                for attr in e.attributes().flatten() {
                    let k = std::str::from_utf8(attr.key.as_ref()).ok()?.to_string();
                    let v = std::str::from_utf8(&attr.value).ok()?.to_string();
                    let attr_path = format!("{}/@{}", path, k);
                    attr_nodes.push(InternalNode {
                        key: format!("@{k}"),
                        path: attr_path,
                        kind: NodeKind::XmlAttr(v),
                    });
                }
                stack.push((tag, path, attr_nodes));
            }
            // Self-closing tags like <item/> — emit as element with no children
            Ok(XmlEvent::Empty(e)) => {
                let tag = std::str::from_utf8(e.name().as_ref()).ok()?.to_string();
                let parent_path = stack.last().map(|(_, p, _)| p.as_str()).unwrap_or("");
                let key = format!("{}/{}", parent_path, tag);
                let idx = *sibling_counts.entry(key.clone()).or_insert(0);
                sibling_counts.insert(key.clone(), idx + 1);
                let path = format!("{}[{}]", key, idx);

                let mut attr_nodes = Vec::new();
                for attr in e.attributes().flatten() {
                    let k = std::str::from_utf8(attr.key.as_ref()).ok()?.to_string();
                    let v = std::str::from_utf8(&attr.value).ok()?.to_string();
                    let attr_path = format!("{}/@{}", path, k);
                    attr_nodes.push(InternalNode {
                        key: format!("@{k}"),
                        path: attr_path,
                        kind: NodeKind::XmlAttr(v),
                    });
                }
                let node = InternalNode {
                    key: tag,
                    path: path.clone(),
                    kind: NodeKind::XmlElement(attr_nodes),
                };
                if let Some((_, _, parent_children)) = stack.last_mut() {
                    parent_children.push(node);
                } else {
                    // Root is a self-closing element
                    return Some(TreeViewerState {
                        roots: vec![node],
                        collapsed: HashSet::new(),
                        mode: 2,
                    });
                }
            }
            Ok(XmlEvent::End(_)) => {
                let (tag, path, children) = stack.pop()?;
                let node = InternalNode {
                    key: tag,
                    path: path.clone(),
                    kind: NodeKind::XmlElement(children),
                };
                if let Some((_, _, parent_children)) = stack.last_mut() {
                    parent_children.push(node);
                } else {
                    return Some(TreeViewerState {
                        roots: vec![node],
                        collapsed: HashSet::new(),
                        mode: 2,
                    });
                }
            }
            Ok(XmlEvent::Text(e)) => {
                // XML entities (e.g. &amp;) are not decoded — quick-xml 0.40 removed unescape()
                let trimmed = String::from_utf8_lossy(e.as_ref()).trim().to_string();
                if !trimmed.is_empty()
                    && let Some((_, parent_path, children)) = stack.last_mut()
                {
                    let text_path = format!("{}/text()", parent_path);
                    children.push(InternalNode {
                        key: String::new(),
                        path: text_path,
                        kind: NodeKind::XmlText(trimmed),
                    });
                }
            }
            Ok(XmlEvent::Eof) | Err(_) => break,
            _ => {}
        }
    }
    // Malformed XML (unclosed tags) or no root element found
    None
}

// ---------------------------------------------------------------------------
// Flatten tree → visible Slint nodes
// ---------------------------------------------------------------------------

fn flatten(
    nodes: &[InternalNode],
    collapsed: &HashSet<String>,
    depth: i32,
    result: &mut Vec<crate::TreeNode>,
) {
    for node in nodes {
        let (node_kind, value, has_children, child_count, children_ref) = match &node.kind {
            NodeKind::JsonObject(ch) => (
                0i32,
                String::new(),
                !ch.is_empty(),
                ch.len() as i32,
                Some(ch.as_slice()),
            ),
            NodeKind::JsonArray(ch) => (
                1i32,
                String::new(),
                !ch.is_empty(),
                ch.len() as i32,
                Some(ch.as_slice()),
            ),
            NodeKind::JsonString(s) => (2i32, s.clone(), false, 0, None),
            NodeKind::JsonNumber(n) => (3i32, n.clone(), false, 0, None),
            NodeKind::JsonBool(b) => (4i32, b.to_string(), false, 0, None),
            NodeKind::JsonNull => (5i32, String::new(), false, 0, None),
            NodeKind::XmlElement(ch) => (
                6i32,
                node.key.clone(),
                !ch.is_empty(),
                ch.len() as i32,
                Some(ch.as_slice()),
            ),
            NodeKind::XmlAttr(v) => (7i32, v.clone(), false, 0, None),
            NodeKind::XmlText(v) => (8i32, v.clone(), false, 0, None),
        };

        let is_expanded = !collapsed.contains(&node.path);

        result.push(crate::TreeNode {
            depth,
            key: node.key.clone().into(),
            value: value.into(),
            node_kind,
            has_children,
            is_expanded: !has_children || is_expanded,
            child_count,
            path: node.path.clone().into(),
        });

        if has_children
            && is_expanded
            && let Some(ch) = children_ref
        {
            flatten(ch, collapsed, depth + 1, result);
        }
    }
}

fn build_flat_list(state: &TreeViewerState) -> Vec<crate::TreeNode> {
    let mut result = Vec::new();
    flatten(&state.roots, &state.collapsed, 0, &mut result);
    result
}

// ---------------------------------------------------------------------------
// Callback registration
// ---------------------------------------------------------------------------

pub(super) fn register_callbacks(window: &crate::AppWindow) {
    let tree_state: Rc<RefCell<Option<TreeViewerState>>> = Rc::new(RefCell::new(None));

    // ── cell-value-selected ─────────────────────────────────────────────────
    {
        let ww = window.as_weak(); // clone required: moved into callback closure
        let state_ref = Rc::clone(&tree_state); // clone required: shared across callbacks
        window
            .global::<crate::UiState>()
            .on_cell_value_selected(move |value| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let text = value.as_str();

                let parsed = if text.is_empty() {
                    None
                } else {
                    parse_json(text).or_else(|| parse_xml(text))
                };

                match parsed {
                    Some(state) => {
                        let flat = build_flat_list(&state);
                        let mode = state.mode;
                        *state_ref.borrow_mut() = Some(state);
                        ui.set_cell_tree_nodes(Rc::new(slint::VecModel::from(flat)).into());
                        ui.set_cell_preview_mode(mode);
                    }
                    None => {
                        *state_ref.borrow_mut() = None;
                        ui.set_cell_preview_mode(0);
                    }
                }
            });
    }

    // ── cell-tree-node-toggle ───────────────────────────────────────────────
    {
        let ww = window.as_weak(); // clone required: moved into callback closure
        let state_ref = Rc::clone(&tree_state); // clone required: shared across callbacks
        window
            .global::<crate::UiState>()
            .on_cell_tree_node_toggle(move |path| {
                let Some(w) = ww.upgrade() else { return };
                let ui = w.global::<crate::UiState>();
                let mut borrow = state_ref.borrow_mut();
                let Some(state) = borrow.as_mut() else { return };

                let path_str = path.to_string();
                if state.collapsed.contains(&path_str) {
                    state.collapsed.remove(&path_str);
                } else {
                    state.collapsed.insert(path_str);
                }
                let flat = build_flat_list(state);
                ui.set_cell_tree_nodes(Rc::new(slint::VecModel::from(flat)).into());
            });
    }

    // ── cell-copy-path ──────────────────────────────────────────────────────
    {
        let ww = window.as_weak(); // clone required: moved into callback closure
        window
            .global::<crate::UiState>()
            .on_cell_copy_path(move |path| {
                let Some(w) = ww.upgrade() else { return };
                w.global::<crate::UiState>().invoke_copy_result_cell(path);
            });
    }

    // ── cell-copy-node-value ────────────────────────────────────────────────
    {
        let ww = window.as_weak(); // clone required: moved into callback closure
        window
            .global::<crate::UiState>()
            .on_cell_copy_node_value(move |value| {
                let Some(w) = ww.upgrade() else { return };
                w.global::<crate::UiState>().invoke_copy_result_cell(value);
            });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_should_return_none_for_primitive_string() {
        assert!(parse_json(r#""hello""#).is_none());
    }

    #[test]
    fn parse_json_should_return_none_for_invalid_json() {
        assert!(parse_json("{not json}").is_none());
    }

    #[test]
    fn parse_json_should_parse_object() {
        let state = parse_json(r#"{"name":"Alice","age":30}"#).unwrap();
        assert_eq!(state.mode, 1);
        assert_eq!(state.roots.len(), 1);
        let NodeKind::JsonObject(children) = &state.roots[0].kind else {
            panic!("not object")
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn parse_json_should_parse_array() {
        let state = parse_json(r#"[1,2,3]"#).unwrap();
        assert_eq!(state.mode, 1);
        let NodeKind::JsonArray(children) = &state.roots[0].kind else {
            panic!("not array")
        };
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn flatten_should_produce_object_then_leaves() {
        let state = parse_json(r#"{"x":1,"y":true}"#).unwrap();
        let flat = build_flat_list(&state);
        assert_eq!(flat.len(), 3); // root object + 2 leaves
        assert_eq!(flat[0].node_kind, 0); // JsonObject
        assert!(flat[0].has_children);
        assert_eq!(flat[1].node_kind, 3); // JsonNumber
        assert_eq!(flat[2].node_kind, 4); // JsonBool
    }

    #[test]
    fn flatten_should_hide_children_when_collapsed() {
        let mut state = parse_json(r#"{"x":1}"#).unwrap();
        state.collapsed.insert(state.roots[0].path.clone());
        let flat = build_flat_list(&state);
        assert_eq!(flat.len(), 1); // only the root object (collapsed)
        assert!(!flat[0].is_expanded);
    }

    #[test]
    fn parse_xml_should_return_none_for_invalid_xml() {
        assert!(parse_xml("<unclosed").is_none());
    }

    #[test]
    fn parse_xml_should_parse_simple_element() {
        let state = parse_xml("<root><child>text</child></root>").unwrap();
        assert_eq!(state.mode, 2);
        let flat = build_flat_list(&state);
        // root element + child element + text node
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].node_kind, 6); // XmlElement root
        assert_eq!(flat[1].node_kind, 6); // XmlElement child
        assert_eq!(flat[2].node_kind, 8); // XmlText
    }

    #[test]
    fn parse_xml_should_include_attributes() {
        let state = parse_xml(r#"<item id="1" name="foo"/>"#).unwrap();
        let flat = build_flat_list(&state);
        // element + 2 attrs
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[1].node_kind, 7); // XmlAttr
        assert_eq!(flat[2].node_kind, 7);
    }

    #[test]
    fn path_should_use_dot_bracket_notation_for_json() {
        let state = parse_json(r#"{"users":[{"id":1}]}"#).unwrap();
        let flat = build_flat_list(&state);
        // paths: "" (root obj) → ".users" (array) → ".users[0]" (obj) → ".users[0].id" (num)
        assert_eq!(flat[1].path.as_str(), ".users");
        assert_eq!(flat[2].path.as_str(), ".users[0]");
        assert_eq!(flat[3].path.as_str(), ".users[0].id");
    }
}
