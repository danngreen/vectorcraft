//! Saved Selections panel → Export Positions… / Import Positions…: a panel's controls as a CSV
//! that a PCB tool (the KiCad side) reads and writes, so the artwork and the board agree on where
//! each control is.
//!
//! CSV: `name,type,x_mm,y_mm,w_mm,h_mm`, the PCB outline first, then one row per control sorted
//! by name; millimetres with 3 decimals from the centre of the first artboard (the panel), y down.
//! - A control is a saved selection; its name is its identity in the CSV. Import falls back to the
//!   first object named so (a warning), which it can only move.
//! - Its position is the centre of its member on the anchor layer (the largest there), else of
//!   its smallest ellipse or rectangle (members only, not shapes inside them), else of its member
//!   on the type layer.
//! - Its type is the name of its member on the type layer: a group placed from the type Library,
//!   which keeps the item's name. A blank type in the file means "keep this one's".
//! - The PCB outline (type `Edge.Cuts`) is the largest unrotated, square-cornered rectangle on the
//!   outline layer. It goes from the artwork to the PCB only: import says when the file's differs.
//! - Import: a row moves its control, all its members, hidden and locked ones too; a changed type
//!   swaps the type-layer group for the Library item; a new row places the type's item and saves
//!   it as a saved selection (a type the Library doesn't have is an error, and a new one is a red
//!   5 mm circle on the anchor layer); a control missing from the file is deleted. One undo step
//!   (a `command.batch` of the commands that do it, which the journal records).
//!
//! The layers and Library come from the params, else the document's own settings (kept by the
//! last export or import, with its CSV's path), else the preferences (`positions*`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_geom::{Point, Rect};

use super::fileio::{absolute_path, read_file_head, write_file};
use super::variablecmds::{csv_escape, parse_datasets_csv};
use super::*;

/// Points per millimetre.
const MM: f64 = 72.0 / 25.4;
const HEADER: [&str; 6] = ["name", "type", "x_mm", "y_mm", "w_mm", "h_mm"];
pub const OUTLINE_TYPE: &str = "Edge.Cuts";
const OUTLINE_NAME: &str = "PCB outline";
const SHAPES: [&str; 3] = ["Ellipse", "Rectangle", "Rounded Rectangle"];
const MARKER_MM: f64 = 5.0;
const MARKER_RED: &str = "#ff0000";
const MAX_CSV_BYTES: usize = 1_048_576;
/// Most rows a file may have (a panel has tens of controls).
const MAX_ROWS: usize = 10_000;
/// The settings' param keys, as the document keeps them.
const KEYS: [&str; 4] = ["anchorLayer", "typeLayer", "outlineLayer", "library"];

// ---- Settings ------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Settings {
    anchor: String,
    kind: String,
    outline: String,
    library: String,
}

impl Settings {
    fn get(&self, key: &str) -> &str {
        match key {
            "anchorLayer" => &self.anchor,
            "typeLayer" => &self.kind,
            "outlineLayer" => &self.outline,
            _ => &self.library,
        }
    }
}

fn pref_settings(s: &Session) -> Settings {
    Settings {
        anchor: s.prefs.positions_anchor_layer.clone(),
        kind: s.prefs.positions_type_layer.clone(),
        outline: s.prefs.positions_outline_layer.clone(),
        library: s.prefs.positions_library.clone(),
    }
}

/// The params' settings, else the document's, else the preferences'.
fn settings(s: &Session, p: &Value) -> Result<Settings> {
    let saved = &s.doc()?.doc.positions;
    let mut out = pref_settings(s);
    for key in KEYS {
        let v = str_param(p, key).or_else(|| saved.get(key).and_then(Value::as_str)).map(str::trim).filter(|v| !v.is_empty());
        if let Some(v) = v {
            let v = v.to_string();
            match key {
                "anchorLayer" => out.anchor = v,
                "typeLayer" => out.kind = v,
                "outlineLayer" => out.outline = v,
                _ => out.library = v,
            }
        }
    }
    Ok(out)
}

/// Keep `path` and the settings that differ from the preferences' with the document, so it reopens
/// on them and follows the preferences for the rest. Not an undo step (like the settings of any
/// export); the document has changed only if they differ.
fn remember(s: &mut Session, set: &Settings, path: &str) -> Result<()> {
    let prefs = pref_settings(s);
    let mut keep = Map::new();
    keep.insert("path".into(), json!(path));
    for key in KEYS {
        if set.get(key) != prefs.get(key) {
            keep.insert(key.into(), json!(set.get(key)));
        }
    }
    let st = s.doc_mut()?;
    if st.doc.positions != keep {
        Arc::make_mut(&mut st.doc).positions = keep;
        st.revision += 1;
    }
    Ok(())
}

// ---- Reading the document ------------------------------------------------------------------

struct Obj<'a> {
    node: &'a Node,
    /// The name of its top-level layer.
    layer: String,
    bounds: Option<Rect>,
}

/// A top-level layer: (id, name, visible, locked).
type LayerRow = (NodeId, String, bool, bool);

/// The document's objects by id with their layer, its layers and saved selections, and the
/// origin: the first artboard's centre.
struct Scan<'a> {
    set: &'a Settings,
    origin: Point,
    objs: HashMap<NodeId, Obj<'a>>,
    /// Every object in paint order (named-object lookups take the first).
    order: Vec<NodeId>,
    layers: Vec<LayerRow>,
    sets: Vec<(String, Vec<NodeId>)>,
}

#[derive(Clone, Debug)]
struct Control {
    name: String,
    ids: Vec<NodeId>,
    x: f64,
    y: f64,
    kind: String,
    /// Its member on the type layer whose name is its type.
    mount: Option<NodeId>,
    /// An object found by its name, not a saved selection.
    object: bool,
}

#[derive(Clone, Copy, Debug)]
struct Outline {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl<'a> Scan<'a> {
    fn new(doc: &'a Document, set: &'a Settings) -> Self {
        let origin = doc.artboards.first().map_or(Point::ZERO, |a| a.rect.center());
        let symbols = doc.symbol_boxes();
        let (mut objs, mut order, mut layers) = (HashMap::new(), Vec::new(), Vec::new());
        for layer in &doc.layers {
            let name = layer.display_name();
            layers.push((layer.id, name.clone(), layer.visible, layer.locked));
            for child in layer.children().into_iter().flatten() {
                child.walk(&mut |n: &'a Node| {
                    order.push(n.id);
                    objs.insert(n.id, Obj { node: n, layer: name.clone(), bounds: n.geometric_bounds_with(&symbols) });
                });
            }
        }
        let sets =
            doc.saved_selections.iter().map(|x| (x.name.clone(), x.objects.iter().copied().filter(|i| objs.contains_key(i)).collect())).collect();
        Scan { set, origin, objs, order, layers, sets }
    }

    fn layer_named(&self, name: &str) -> Option<&LayerRow> {
        self.layers.iter().find(|l| same_name(&l.1, name))
    }

    fn on<'b>(&self, layer: &str, ids: &'b [NodeId]) -> Vec<&'b NodeId> {
        ids.iter().filter(|i| self.objs.get(i).is_some_and(|o| same_name(&o.layer, layer))).collect()
    }

    fn bounds(&self, id: NodeId) -> Option<Rect> {
        self.objs.get(&id).and_then(|o| o.bounds)
    }

    /// The member whose centre is the control's position.
    fn anchor(&self, ids: &[NodeId]) -> Option<NodeId> {
        let with: Vec<NodeId> = ids.iter().copied().filter(|i| self.bounds(*i).is_some()).collect();
        let area = |i: &NodeId| self.bounds(*i).map_or(0.0, |b| b.area());
        if let Some(i) = self.on(&self.set.anchor, &with).into_iter().max_by(|a, b| area(a).total_cmp(&area(b))) {
            return Some(*i);
        }
        let shapes = with.iter().filter(|i| self.objs.get(i).is_some_and(|o| SHAPES.contains(&o.node.kind_label())));
        if let Some(i) = shapes.min_by(|a, b| area(a).total_cmp(&area(b))) {
            return Some(*i);
        }
        self.on(&self.set.kind, &with).first().map(|i| **i)
    }

    fn to_mm(&self, p: Point) -> (f64, f64) {
        ((p.x - self.origin.x) / MM, (p.y - self.origin.y) / MM)
    }

    fn to_pt(&self, x: f64, y: f64) -> Point {
        Point::new(x * MM + self.origin.x, y * MM + self.origin.y)
    }

    fn control(&self, name: &str, ids: &[NodeId], warnings: &mut Vec<String>) -> Option<Control> {
        let Some(a) = self.anchor(ids) else {
            warnings.push(format!(
                "{name}: no member to take its position from (nothing on the {} or {} layer, no circle or rectangle)",
                self.set.anchor, self.set.kind
            ));
            return None;
        };
        let (x, y) = self.to_mm(self.bounds(a)?.center());
        let named: Vec<(NodeId, &str)> = self
            .on(&self.set.kind, ids)
            .into_iter()
            .filter_map(|i| self.objs.get(i).and_then(|o| o.node.name.as_deref()).filter(|n| !n.trim().is_empty()).map(|n| (*i, n)))
            .collect();
        match named.as_slice() {
            [] => warnings.push(format!("{name}: no named group on the {} layer, so no type", self.set.kind)),
            [(_, first), rest @ ..] if rest.iter().any(|(_, n)| n != first) => {
                warnings.push(format!("{name}: several named groups on the {} layer; using the first", self.set.kind))
            }
            _ => {}
        }
        let (mount, kind) = named.first().map_or((None, String::new()), |(i, n)| (Some(*i), n.to_string()));
        Some(Control { name: name.to_string(), ids: ids.to_vec(), x, y, kind, mount, object: false })
    }

    fn controls(&self, warnings: &mut Vec<String>) -> Vec<Control> {
        self.sets.iter().filter_map(|(name, ids)| self.control(name, ids, warnings)).collect()
    }

    /// The PCB outline: the largest rectangle on the outline layer, groups included.
    fn outline(&self, warnings: &mut Vec<String>) -> Option<Outline> {
        self.layer_named(&self.set.outline)?;
        let (mut best, mut skipped): (Option<Rect>, usize) = (None, 0);
        for i in &self.order {
            let Some(o) = self.objs.get(i).filter(|o| same_name(&o.layer, &self.set.outline)) else { continue };
            match rect_of(o.node) {
                Some(r) if best.is_none_or(|b| r.area() > b.area()) => best = Some(r),
                Some(_) => {}
                None if matches!(o.node.kind_label(), "Rectangle" | "Rounded Rectangle") => skipped += 1,
                None => {}
            }
        }
        if skipped > 0 {
            warnings.push(format!(
                "{} layer: {skipped} rotated or round-cornered rectangle{} can't be the PCB outline, so skipped",
                self.set.outline,
                if skipped == 1 { "" } else { "s" }
            ));
        }
        let Some(r) = best else {
            warnings.push(format!("{} layer: no rectangle, so no PCB outline", self.set.outline));
            return None;
        };
        let (x, y) = self.to_mm(r.center());
        Some(Outline { x, y, w: r.width() / MM, h: r.height() / MM })
    }

    /// The first object (not a layer) named `name`, in paint order.
    fn named(&self, name: &str) -> Option<NodeId> {
        self.order.iter().copied().find(|i| self.objs.get(i).is_some_and(|o| !o.node.is_layer() && o.node.name.as_deref() == Some(name)))
    }
}

/// The rectangle a path is, if it is one with square corners and sides along the axes (a
/// Rectangle, or a path of four corners drawn as one): one closed subpath of four straight
/// corners.
fn rect_of(n: &Node) -> Option<Rect> {
    let path = n.path_data()?;
    let [sp] = path.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() != 4 || sp.anchors.iter().any(|a| a.h_in != a.p || a.h_out != a.p) {
        return None;
    }
    let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
    if pts.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return None;
    }
    let first = *pts.first()?;
    let r = pts.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
    let tol = 1e-6 * r.width().max(r.height()).max(1.0);
    let straight = pts.iter().zip(pts.iter().cycle().skip(1)).all(|(a, b)| (a.x - b.x).abs() <= tol || (a.y - b.y).abs() <= tol);
    (straight && r.width() > tol && r.height() > tol).then_some(r)
}

// ---- The CSV -------------------------------------------------------------------------------

/// A number as the file has it: 3 decimals, no negative zero.
fn mm3(v: f64) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" { "0.000".into() } else { s }
}

/// A position as the report shows it: 1 decimal, no negative zero.
fn mm1(v: f64) -> String {
    let s = format!("{v:.1}");
    if s == "-0.0" { "0.0".into() } else { s }
}

fn at(x: f64, y: f64) -> String {
    format!("{}, {}", mm1(x), mm1(y))
}

fn outline_text(o: &Outline) -> String {
    format!("{} x {}mm at {}", mm1(o.w), mm1(o.h), at(o.x, o.y))
}

/// The digits of a name compare as numbers, so `Jack 2` comes before `Jack 10`.
fn natural_key(s: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            let mut digits = c.to_string();
            while let Some(d) = chars.next_if(char::is_ascii_digit) {
                digits.push(d);
            }
            // Zero-padded, so a longer number sorts after a shorter one.
            out.push((false, format!("{:0>40}", digits.trim_start_matches('0'))));
        } else {
            out.push((true, c.to_lowercase().collect()));
        }
    }
    out
}

fn to_csv(controls: &[Control], outline: Option<&Outline>) -> String {
    let mut out = HEADER.join(",");
    out.push('\n');
    if let Some(o) = outline {
        out.push_str(&[OUTLINE_NAME.into(), OUTLINE_TYPE.into(), mm3(o.x), mm3(o.y), mm3(o.w), mm3(o.h)].join(","));
        out.push('\n');
    }
    let mut sorted: Vec<&Control> = controls.iter().collect();
    sorted.sort_by_cached_key(|c| natural_key(&c.name));
    for c in sorted {
        out.push_str(&[csv_escape(&c.name), csv_escape(&c.kind), mm3(c.x), mm3(c.y), String::new(), String::new()].join(","));
        out.push('\n');
    }
    out
}

struct Rows {
    /// name → (type, x, y), in file order.
    rows: Vec<(String, String, f64, f64)>,
    outline: Option<Outline>,
}

fn read_rows(text: &str, cmd: &str, errors: &mut Vec<String>) -> Result<Rows> {
    let cells = parse_datasets_csv(text, cmd)?;
    let header: Vec<String> = cells.first().map(|(_, h)| h.iter().take(4).map(|c| c.trim().to_lowercase()).collect()).unwrap_or_default();
    if header != HEADER[..4] {
        return Err(bad(cmd, format!("the first line must be {}", HEADER.join(","))));
    }
    if cells.len() > MAX_ROWS + 1 {
        return Err(bad(cmd, format!("more than {MAX_ROWS} rows")));
    }
    let (mut rows, mut outline, mut seen) = (Vec::new(), None, HashSet::new());
    for (line, row) in cells.iter().skip(1) {
        let cell = |k: usize| row.get(k).map_or("", |c| c.trim());
        if (0..row.len()).all(|k| cell(k).is_empty()) {
            continue;
        }
        let (name, kind) = (cell(0), cell(1));
        let num = |k: usize| cell(k).parse::<f64>().ok().filter(|v| v.is_finite() && v.abs() < 1e5);
        let (Some(x), Some(y)) = (num(2), num(3)) else {
            errors.push(format!("line {line}: {}: x_mm and y_mm must be numbers", if name.is_empty() { "(no name)" } else { name }));
            continue;
        };
        if kind == OUTLINE_TYPE {
            match (num(4).filter(|w| *w > 0.0), num(5).filter(|h| *h > 0.0)) {
                (Some(w), Some(h)) if outline.is_none() => outline = Some(Outline { x, y, w, h }),
                (Some(_), Some(_)) => errors.push(format!("line {line}: a second PCB outline; the first is used")),
                _ => errors.push(format!("line {line}: the PCB outline needs w_mm and h_mm, positive numbers")),
            }
            continue;
        }
        if name.is_empty() {
            errors.push(format!("line {line}: no name"));
        } else if !seen.insert(name.to_string()) {
            errors.push(format!("line {line}: {name} appears twice; the first row is used"));
        } else {
            rows.push((name.to_string(), kind.to_string(), x, y));
        }
    }
    Ok(Rows { rows, outline })
}

fn csv_text(p: &Value, cmd: &str, path: Option<&str>) -> Result<String> {
    let bytes = match (path, str_param(p, "csv")) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), _) => read_file_head(&absolute_path(path), MAX_CSV_BYTES + 1)?,
        (None, Some(csv)) => csv.as_bytes().to_vec(),
        (None, None) => return Err(bad(cmd, "give `path` or `csv` (or export once, so the document knows its file)")),
    };
    if bytes.len() > MAX_CSV_BYTES {
        return Err(bad(cmd, format!("the CSV is larger than {MAX_CSV_BYTES} bytes")));
    }
    String::from_utf8(bytes).map_err(|_| bad(cmd, "the CSV isn't UTF-8"))
}

// ---- Import --------------------------------------------------------------------------------

#[derive(Default)]
struct Report {
    errors: Vec<String>,
    removed: Vec<String>,
    retyped: Vec<String>,
    moved: Vec<String>,
    added: Vec<String>,
    warnings: Vec<String>,
}

impl Report {
    fn changes(&self) -> usize {
        self.removed.len() + self.retyped.len() + self.moved.len() + self.added.len()
    }

    fn json(&self) -> Value {
        json!({
            "errors": self.errors, "removed": self.removed, "retyped": self.retyped,
            "moved": self.moved, "added": self.added, "warnings": self.warnings, "changes": self.changes(),
        })
    }
}

fn direction(dx: f64, dy: f64) -> String {
    let mut parts = vec![];
    if dy.abs() >= 0.0005 {
        parts.push(format!("{} {}mm", if dy > 0.0 { "down" } else { "up" }, mm1(dy.abs())));
    }
    if dx.abs() >= 0.0005 {
        parts.push(format!("{} {}mm", if dx > 0.0 { "right" } else { "left" }, mm1(dx.abs())));
    }
    parts.join(", ")
}

fn same(a: f64, b: f64) -> bool {
    (a * 1000.0).round() == (b * 1000.0).round()
}

/// The names of the Library's graphics, or None when there's no such Library.
fn library_types(s: &mut Session, library: &str) -> Option<HashSet<String>> {
    let lib = s.execute("library.get", &json!({ "library": library })).ok()?;
    if !lib.get("name").and_then(Value::as_str).is_some_and(|n| same_name(n, library)) && lib.get("id").and_then(Value::as_str) != Some(library) {
        return None;
    }
    Some(
        lib.get("graphics")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|g| g.get("name").and_then(Value::as_str))
            .map(String::from)
            .collect(),
    )
}

/// What an import of `text` does: the report, and the commands that do it.
fn plan(s: &mut Session, set: &Settings, text: &str, cmd: &str) -> Result<(Report, Vec<Value>)> {
    let mut r = Report::default();
    let rows = read_rows(text, cmd, &mut r.errors)?;
    let types = library_types(s, &set.library);
    let doc = s.doc()?.doc.clone();
    let scan = Scan::new(&doc, set);
    let mut controls = scan.controls(&mut r.warnings);
    let known: HashSet<String> = controls.iter().map(|c| c.name.clone()).collect();
    // Rows that name no saved selection try the first object with that name.
    for (name, ..) in &rows.rows {
        if known.contains(name) {
            continue;
        }
        if let Some(id) = scan.named(name)
            && let Some(mut c) = scan.control(name, &[id], &mut vec![])
        {
            let kind = scan.objs.get(&id).map_or("object", |o| o.node.kind_label()).to_lowercase();
            r.warnings.push(format!("{name}: no saved selection by that name; using the {kind} named so"));
            c.object = true;
            controls.push(c);
        }
    }
    controls.sort_by_cached_key(|c| natural_key(&c.name));
    let by_name: HashMap<&str, &(String, String, f64, f64)> = rows.rows.iter().map(|r| (r.0.as_str(), r)).collect();
    let has_type = |k: &str| types.as_ref().is_some_and(|t| t.contains(k));
    let no_library = types.is_none();
    if no_library {
        r.errors.push(format!("there's no Library named {}, so no type can change and new controls are red markers", set.library));
    }
    let (mut remove, mut retype, mut moves, mut add) = (vec![], vec![], vec![], vec![]);
    for c in &controls {
        let Some((_, kind, x, y)) = by_name.get(c.name.as_str()) else {
            if !c.object {
                r.removed.push(format!("{}: {} at {}", c.name, if c.kind.is_empty() { "(no type)" } else { &c.kind }, at(c.x, c.y)));
                remove.push(c);
            }
            continue;
        };
        if !kind.is_empty() && *kind != c.kind {
            if c.object {
                r.warnings.push(format!("{}: an object, not a saved selection, so its type isn't checked", c.name));
            } else if !has_type(kind) {
                if !no_library {
                    r.errors.push(format!("{}: type {kind} isn't in the Library {}; its type stays {}", c.name, set.library, or_none(&c.kind)));
                }
            } else if c.mount.is_none() {
                r.errors.push(format!("{}: has no group on the {} layer to swap for {kind}", c.name, set.kind));
            } else {
                r.retyped.push(format!("{}: {} => {kind}", c.name, or_none(&c.kind)));
                retype.push((c, kind.clone()));
            }
        }
        if !(same(*x, c.x) && same(*y, c.y)) {
            let label = if c.kind.is_empty() { kind.as_str() } else { c.kind.as_str() };
            let label = if label.is_empty() { String::new() } else { format!("{label} ") };
            r.moved.push(format!("{}: {label}moved from {} to {} ({})", c.name, at(c.x, c.y), at(*x, *y), direction(x - c.x, y - c.y)));
            moves.push((c, x - c.x, y - c.y));
        }
    }
    let present: HashSet<&str> = controls.iter().map(|c| c.name.as_str()).collect();
    let mut new_rows: Vec<&(String, String, f64, f64)> = rows.rows.iter().filter(|r| !present.contains(r.0.as_str())).collect();
    new_rows.sort_by_cached_key(|r| natural_key(&r.0));
    for (name, kind, x, y) in new_rows {
        if has_type(kind) {
            r.added.push(format!("{name}: {kind} at {}", at(*x, *y)));
            add.push((name, kind, *x, *y, false));
        } else {
            if !no_library {
                r.errors.push(format!("{name}: type {} isn't in the Library {}; a red marker goes there instead", or_blank(kind), set.library));
            }
            r.added.push(format!("{name}: red marker at {}", at(*x, *y)));
            add.push((name, kind, *x, *y, true));
        }
    }
    let needs_type_layer = !retype.is_empty() || add.iter().any(|a| !a.4);
    if needs_type_layer && scan.layer_named(&set.kind).is_none() {
        return Err(bad(cmd, format!("the document has no layer named {}, where the Library's groups go", set.kind)));
    }
    if let Some(file) = rows.outline {
        // The outline goes from the artwork to the PCB only: tell, don't change.
        let ours = scan.outline(&mut vec![]);
        let differs = ours.is_none_or(|o| !(same(o.x, file.x) && same(o.y, file.y) && same(o.w, file.w) && same(o.h, file.h)));
        if differs {
            r.warnings.push(format!(
                "the file's PCB outline ({}) isn't the {} layer's ({}); the artwork keeps its own, and the next export sends it to the PCB",
                outline_text(&file),
                set.outline,
                ours.as_ref().map_or_else(|| "none".to_string(), outline_text)
            ));
        }
    }
    let cmds = commands(&scan, set, &remove, &retype, &moves, &add);
    Ok((r, cmds))
}

fn or_none(s: &str) -> &str {
    if s.is_empty() { "(none)" } else { s }
}

fn or_blank(s: &str) -> &str {
    if s.is_empty() { "(blank)" } else { s }
}

/// The commands of an import, as one batch.
fn commands(
    scan: &Scan,
    set: &Settings,
    remove: &[&Control],
    retype: &[(&Control, String)],
    moves: &[(&Control, f64, f64)],
    add: &[(&String, &String, f64, f64, bool)],
) -> Vec<Value> {
    let mut cmds = vec![];
    let mut c = |command: &str, params: Value| cmds.push(json!({ "command": command, "params": params }));
    // Art goes onto the type layer (Library items) and the anchor layer (markers): unlock and show
    // them for the import, and put them back after.
    let mount = scan.layer_named(&set.kind);
    let marker_layer = scan.layer_named(&set.anchor).or_else(|| scan.layers.iter().find(|l| l.2)).or(mount);
    let mut used: Vec<&LayerRow> = vec![];
    if !retype.is_empty() || add.iter().any(|a| !a.4) {
        used.extend(mount);
    }
    if add.iter().any(|a| a.4) {
        used.extend(marker_layer.filter(|m| !used.iter().any(|u| u.0 == m.0)));
    }
    for (id, _, visible, locked) in &used {
        if *locked || !*visible {
            c("layer.setProps", json!({ "ids": [id.0], "locked": false, "visible": true }));
        }
    }
    // A control's members are selected by id, hidden and locked ones too.
    for ctl in remove {
        c("select.set", json!({ "ids": ids(&ctl.ids) }));
        c("edit.clear", json!({}));
        c("select.editSaved", json!({ "name": ctl.name, "delete": true }));
    }
    // Moves first, while every member still has its id; a retyped control's new group then goes
    // where the old one has moved to.
    let mut moved = HashMap::new();
    for (ctl, dx, dy) in moves {
        c("select.set", json!({ "ids": ids(&ctl.ids) }));
        c("object.move", json!({ "dx": dx * MM, "dy": dy * MM }));
        moved.insert(ctl.name.as_str(), (dx * MM, dy * MM));
    }
    for (ctl, kind) in retype {
        let (Some(old), Some(mount)) = (ctl.mount, mount) else { continue };
        let Some(b) = scan.bounds(old) else { continue };
        let (dx, dy) = moved.get(ctl.name.as_str()).copied().unwrap_or((0.0, 0.0));
        let center = b.center();
        c("edit.clear", json!({ "ids": [old.0] }));
        c("layer.setCurrent", json!({ "id": mount.0 .0 }));
        c("library.use", json!({ "library": set.library, "kind": "graphic", "item": kind, "center": [center.x + dx, center.y + dy] }));
        // The placed item is selected: add the other members and save the set again over itself.
        let rest: Vec<u64> = ctl.ids.iter().filter(|i| **i != old).map(|i| i.0).collect();
        if !rest.is_empty() {
            c("select.add", json!({ "ids": rest }));
        }
        c("select.save", json!({ "name": ctl.name }));
    }
    for (name, kind, x, y, marker) in add {
        let p = scan.to_pt(*x, *y);
        if *marker {
            let Some(layer) = marker_layer else { continue };
            let d = MARKER_MM * MM;
            c("layer.setCurrent", json!({ "id": layer.0 .0 }));
            c("shape.ellipse", json!({ "x": p.x - d / 2.0, "y": p.y - d / 2.0, "width": d, "height": d }));
            c("paint.setFill", json!({ "color": MARKER_RED }));
            c("paint.setStroke", json!({ "none": true }));
            c("attributes.set", json!({ "note": format!("Type not in the Library: {}", or_blank(kind)) }));
        } else if let Some(mount) = mount {
            c("layer.setCurrent", json!({ "id": mount.0 .0 }));
            c("library.use", json!({ "library": set.library, "kind": "graphic", "item": kind, "center": [p.x, p.y] }));
        }
        c("select.save", json!({ "name": name }));
    }
    for (id, _, visible, locked) in &used {
        if *locked || !*visible {
            c("layer.setProps", json!({ "ids": [id.0], "locked": locked, "visible": visible }));
        }
    }
    if !cmds.is_empty() {
        cmds.push(json!({ "command": "select.none", "params": {} }));
    }
    cmds
}

fn ids(v: &[NodeId]) -> Vec<u64> {
    v.iter().map(|i| i.0).collect()
}

/// The file to use: `path`, else the one the document last used.
fn path_param(s: &Session, p: &Value) -> Result<Option<String>> {
    if let Some(path) = str_param(p, "path") {
        return Ok(Some(path.to_string()));
    }
    if p.get("csv").is_some() {
        return Ok(None);
    }
    Ok(s.doc()?.doc.positions.get("path").and_then(Value::as_str).map(String::from))
}

// ---- Commands ------------------------------------------------------------------------------

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "positions.export";
    let set = settings(s, p)?;
    let path = str_param(p, "path").map(absolute_path);
    let doc = s.doc()?.doc.clone();
    let scan = Scan::new(&doc, &set);
    let mut warnings = vec![];
    let controls = scan.controls(&mut warnings);
    let outline = scan.outline(&mut warnings);
    let csv = to_csv(&controls, outline.as_ref());
    if csv.len() > MAX_CSV_BYTES {
        return Err(bad(C, format!("the CSV would be larger than {MAX_CSV_BYTES} bytes")));
    }
    let mut out = json!({ "controls": controls.len(), "outline": outline.is_some(), "warnings": warnings });
    match path {
        Some(path) => {
            write_file(&path, csv.as_bytes())?;
            remember(s, &set, &path)?;
            out["path"] = json!(path);
        }
        None => out["csv"] = json!(csv),
    }
    Ok(out)
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "positions.check";
    let set = settings(s, p)?;
    let path = path_param(s, p)?;
    let text = csv_text(p, C, path.as_deref())?;
    let (report, _) = plan(s, &set, &text, C)?;
    Ok(report.json())
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "positions.import";
    let set = settings(s, p)?;
    let path = path_param(s, p)?.map(|p| absolute_path(&p));
    let text = csv_text(p, C, path.as_deref())?;
    let (report, cmds) = plan(s, &set, &text, C)?;
    if report.changes() > 0 {
        let current = s.doc()?.current_layer();
        s.execute("command.batch", &json!({ "label": "Import Positions", "commands": cmds }))?;
        // The layer that was current stays so, unless the import removed it.
        if let Some(layer) = current {
            let _ = s.execute("layer.setCurrent", &json!({ "id": layer.0 }));
        }
    }
    if let Some(path) = &path {
        remember(s, &set, path)?;
    }
    Ok(report.json())
}

fn show_settings(s: &mut Session, p: &Value) -> Result<Value> {
    current_settings(s, p)
}

/// The settings Export/Import Positions would use with params `p` (`positions.settings`).
pub fn current_settings(s: &Session, p: &Value) -> Result<Value> {
    let set = settings(s, p)?;
    let doc = &s.doc()?.doc.positions;
    Ok(json!({
        "anchorLayer": set.anchor, "typeLayer": set.kind, "outlineLayer": set.outline, "library": set.library,
        "path": doc.get("path").cloned().unwrap_or(Value::Null), "document": doc,
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "positions.export",
            "Export Positions…",
            ["Window", "Saved Selections"],
            None,
            "{path?, anchorLayer?, typeLayer?, outlineLayer? (default: the document's settings, else prefs positionsAnchorLayer, positionsTypeLayer, positionsOutlineLayer)} write the saved selections' positions as CSV (name,type,x_mm,y_mm,w_mm,h_mm: mm from the first artboard's centre, y down; the PCB outline first, type Edge.Cuts); with path, the document remembers it and the settings → {path, controls, outline: bool, warnings}; without path → {csv, controls, outline, warnings}",
            has_doc,
            export
        ),
        cmd!(
            query "positions.check",
            "Check Positions",
            [],
            None,
            "{path? | csv? (default: the CSV the document last used), anchorLayer?, typeLayer?, outlineLayer?, library? (as positions.import)} what positions.import would do, changing nothing → {errors, removed, retyped, moved, added, warnings: [text], changes}",
            has_doc,
            check
        ),
        // Not journaled itself: the batch it runs is, so a replay does the same without the file.
        cmd!(
            query "positions.import",
            "Import Positions…",
            ["Window", "Saved Selections"],
            None,
            "{path? | csv? (default: the CSV the document last used), anchorLayer?, typeLayer?, outlineLayer?, library? (default: the document's settings, else prefs positions*)} move, retype, add and remove the saved selections (controls) to match a positions CSV, in one undo step; with path, the document remembers it and the settings → {errors, removed, retyped, moved, added, warnings: [text], changes}",
            has_doc,
            import
        ),
        cmd!(
            query "positions.settings",
            "Positions Settings",
            [],
            None,
            "{anchorLayer?, typeLayer?, outlineLayer?, library?} the settings Export/Import Positions would use (params, else the document's, else prefs) → {anchorLayer, typeLayer, outlineLayer, library, path: the CSV the document last used|null, document: its own settings}",
            has_doc,
            show_settings
        ),
    ]
}
