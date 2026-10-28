//! Saved Selections panel → Export Positions… / Import Positions…: the CSV file, the layers and
//! the Library (`positions.settings`: the document's, else the preferences'), and, for an import,
//! the report of what it will change (`positions.check`) before OK applies it in one undo step.
//! When the file both removes and adds controls, Renamed Items… pairs them up as renames.

use std::path::Path;

use serde_json::{Value, json};

use super::{Dialog, DialogResult, DialogSpec, form};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const EXPORT: &str = "positionsExport";
pub const IMPORT: &str = "positionsImport";
pub const RENAMES: &str = "positionsRenames";

/// The settings fields, as the commands name them, with their labels (the preferences').
const SETTINGS: [(&str, &str); 4] =
    [("anchorLayer", "Position Layer"), ("typeLayer", "Type Layer"), ("outlineLayer", "PCB Outline Layer"), ("library", "Type Library")];

/// The fields a dialog opens with: the CSV the document last used (or none) and its settings.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    let mut fields = vectorcraft_engine::cmd::positions::current_settings(&app.session, &json!({})).unwrap_or_else(|_| json!({}));
    if fields.get("path").is_none_or(Value::is_null) {
        fields["path"] = json!("");
    }
    if let Some(f) = fields.as_object_mut() {
        f.remove("document");
    }
    fields
}

/// Open Export (`import` false) or Import Positions on the active document.
pub(crate) fn open(app: &mut VectorcraftApp, import: bool) {
    app.ui.dialog = Some(Dialog::new(if import { IMPORT } else { EXPORT }, fields(app)));
}

fn import(d: &Dialog) -> bool {
    d.kind == IMPORT
}

/// The command params of the dialog's fields.
fn params(d: &Dialog) -> Value {
    let mut p = json!({ "path": d.str("path") });
    for (key, _) in SETTINGS {
        if import(d) || key != "library" {
            p[key] = json!(d.str(key));
        }
    }
    // The renames chosen in Renamed Items…; until then, the import pairs them by position.
    if import(d)
        && let Some(r) = d.fields.get("renames")
    {
        p["renames"] = r.clone();
    }
    p
}

pub(super) const EXPORT_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Export Positions").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 440.0,
    max_width: Some(560.0),
    ok_enabled: Some(|_, d| !d.str("path").trim().is_empty()),
    ..DialogSpec::FORM
};

pub(super) const IMPORT_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Import Positions").into(),
    body,
    confirm,
    ok: Some("Import"),
    min_width: 440.0,
    max_width: Some(560.0),
    ok_enabled: Some(|_, d| d.fields.get("report").is_some_and(|r| r["changes"].as_u64().unwrap_or(0) > 0)),
    ..DialogSpec::FORM
};

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    widgets::dim_label(
        ui,
        if import(d) {
            tl!("Moves, retypes, adds and removes controls (saved selections) to match a positions CSV, in one undo step.")
        } else {
            tl!("Writes each saved selection's position, and the PCB outline, as a CSV for the PCB.")
        },
    );
    ui.add_space(8.0);
    file_row(app, ui, d, &t);
    ui.add_space(6.0);
    egui::Grid::new("positions-settings").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        for (key, label) in SETTINGS {
            if import(d) || key != "library" {
                form::field(ui, d, key, label);
            }
        }
    });
    if import(d) {
        ui.add_space(8.0);
        check(app, d);
        report(ui, d, &t);
        let mut renames = false;
        ui.horizontal(|ui| {
            if !d.str("path").is_empty() && ui.button(tl!("Check Again")).clicked() {
                // The file may have changed on disk since: read it again.
                d.fields.remove("checked");
            }
            let offered = d.fields.get("report").is_some_and(|r| r["rename"].is_object());
            renames = offered && ui.button(tl!("Renamed Items…")).clicked();
        });
        if renames {
            *d = over_import(d);
        }
    }
    false
}

/// The CSV's path, and Choose… for another.
fn file_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, t: &Tokens) {
    ui.horizontal(|ui| {
        form::row_label(ui, "File");
        let path = d.str("path");
        let name = Path::new(&path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let shown = if path.is_empty() { tl!("No file chosen").to_string() } else { name };
        let color = if path.is_empty() { t.text_dim } else { t.text_strong };
        ui.add(egui::Label::new(egui::RichText::new(shown).color(color)).truncate()).on_hover_text(&path);
        if ui.button(tl!("Choose…")).clicked() {
            let r = crate::picks::in_dialog(app, d, choose);
            if let Err(e) = r {
                app.status(e);
            }
        }
    });
}

/// Pick the CSV: an existing one to import, or where to export.
fn choose(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let current = d.str("path");
    let doc_dir =
        app.session.active().and_then(|st| st.path.as_deref()).and_then(|p| Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned());
    let folder = Path::new(&current).parent().map(|p| p.to_string_lossy().into_owned()).filter(|f| !f.is_empty()).or(doc_dir);
    let name = Path::new(&current).file_name().map_or_else(|| "panel-positions.csv".to_string(), |n| n.to_string_lossy().into_owned());
    let pick = crate::FilePick { name, folder, filters: vec![crate::FilePick::filter("CSV", &["csv"])] };
    let path = if import(d) { crate::picks::open(app, &pick) } else { crate::picks::save(app, &pick) };
    if let Some(path) = path {
        // Renames chosen for another file don't carry over.
        if path != current {
            d.fields.remove("renames");
        }
        d.fields.insert("path".into(), json!(path));
    }
    Ok(())
}

/// Run `positions.check` when the file or a setting changed since the last check.
fn check(app: &mut VectorcraftApp, d: &mut Dialog) {
    let p = params(d);
    let key = p.to_string();
    if d.str("checked") == key {
        return;
    }
    d.fields.insert("checked".into(), json!(key));
    d.fields.remove("report");
    d.fields.remove("error");
    if d.str("path").is_empty() {
        return;
    }
    match app.run("positions.check", p) {
        Ok(r) => {
            d.fields.insert("report".into(), r);
        }
        Err(e) => {
            d.fields.insert("error".into(), json!(e));
        }
    }
}

fn lines<'r>(r: &'r Value, key: &str) -> impl Iterator<Item = &'r str> {
    r[key].as_array().into_iter().flatten().filter_map(Value::as_str)
}

/// What the import will change, errors and removals first, in red.
fn report(ui: &mut egui::Ui, d: &Dialog, t: &Tokens) {
    if let Some(e) = d.fields.get("error").and_then(Value::as_str) {
        ui.label(egui::RichText::new(e).color(t.error));
        return;
    }
    let Some(r) = d.fields.get("report") else {
        widgets::dim_label(ui, tl!("Choose the CSV to import."));
        return;
    };
    let sections = [
        ("errors", tl!("Errors ({count})"), t.error),
        ("removed", tl!("Will be removed ({count})"), t.error),
        ("renamed", tl!("Will be renamed ({count})"), t.text_strong),
        ("retyped", tl!("Will change type ({count})"), t.text_strong),
        ("moved", tl!("Will move ({count})"), t.text_strong),
        ("added", tl!("Will be added ({count})"), t.text_strong),
        ("warnings", tl!("Warnings ({count})"), t.warning),
    ];
    egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
        for (key, heading, color) in sections {
            let items: Vec<&str> = lines(r, key).collect();
            if items.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            let heading = crate::i18n::fmt(heading, &[("count", &items.len().to_string())]);
            ui.label(egui::RichText::new(heading).font(crate::theme::semibold(12.5)).color(color));
            ui.indent(key, |ui| {
                for line in items {
                    ui.add(egui::Label::new(egui::RichText::new(line).size(12.0).color(t.text)).wrap());
                }
            });
        }
    });
    if r["changes"].as_u64() == Some(0) {
        ui.add_space(6.0);
        widgets::dim_label(ui, tl!("Nothing to change: the controls match the file."));
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let id = if import(d) { "positions.import" } else { "positions.export" };
    let r = super::run_and_close(app, id, params(d))?;
    let file = Path::new(&d.str("path")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let message = if import(d) {
        let count = r["changes"].as_u64().unwrap_or(0).to_string();
        crate::i18n::fmt(tl!("Imported {count} changes from {file} (Edit › Undo reverses them)"), &[("count", &count), ("file", &file)])
    } else {
        let count = r["controls"].as_u64().unwrap_or(0).to_string();
        crate::i18n::fmt(tl!("Exported {count} controls to {file}"), &[("count", &count), ("file", &file)])
    };
    app.status(message);
    Ok(r)
}

// ---- Renamed Items -------------------------------------------------------------------------

pub(super) const RENAMES_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Renamed Items").into(),
    body: renames_body,
    confirm: renames_confirm,
    min_width: 600.0,
    max_width: Some(720.0),
    ..DialogSpec::FORM
};

/// Renamed Items over the Import Positions dialog `parent`: the controls its file removes and
/// adds (`report.rename`), and the pairs of them to rename. OK goes back to `parent` with the
/// pairs as its `renames`; Cancel goes back to it as it was.
fn over_import(parent: &Dialog) -> Dialog {
    let r = parent.fields.get("report").map(|r| r["rename"].clone()).unwrap_or_default();
    let mut d = Dialog::new(
        RENAMES,
        json!({ "added": r["added"], "removed": r["removed"], "pairs": r["pairs"], "__added": "", "__removed": "", "__pair": "" }),
    );
    d.fields.insert(super::PARENT.into(), serde_json::to_value(parent).unwrap_or_default());
    d
}

/// The (from, to) pairs to rename.
fn pairs(d: &Dialog) -> Vec<(String, String)> {
    let s = |p: &Value, k: &str| p[k].as_str().map(String::from);
    d.fields.get("pairs").and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((s(p, "from")?, s(p, "to")?))).collect()
}

/// A column's items, (name, detail): those of `key` not in a pair.
fn unpaired(d: &Dialog, key: &str, paired: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let items = d.fields.get(key).and_then(Value::as_array).into_iter().flatten();
    items
        .filter_map(|i| Some((i["name"].as_str()?.to_string(), i["detail"].as_str().unwrap_or_default().to_string())))
        .filter(|(name, _)| !paired(name))
        .collect()
}

fn detail(d: &Dialog, key: &str, name: &str) -> String {
    let items = d.fields.get(key).and_then(Value::as_array).into_iter().flatten();
    items.into_iter().find(|i| i["name"].as_str() == Some(name)).and_then(|i| i["detail"].as_str()).unwrap_or_default().to_string()
}

/// One column: its heading and a list of `items` (shown, hover text, id), `field` holding the one
/// selected. Clicking the selected one deselects it.
fn column(ui: &mut egui::Ui, d: &mut Dialog, t: &Tokens, heading: &str, field: &str, items: &[(String, String, String)]) {
    ui.label(egui::RichText::new(tl!(heading)).font(crate::theme::semibold(12.5)).color(t.text_strong));
    let current = d.str(field);
    let mut pick = None;
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(field).min_scrolled_height(220.0).max_height(220.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(220.0);
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (shown, hover, id) in items {
                    if ui.selectable_label(*id == current, shown).on_hover_text(hover).clicked() {
                        pick = Some(if *id == current { String::new() } else { id.clone() });
                    }
                }
            });
        });
    });
    if let Some(id) = pick {
        d.fields.insert(field.into(), json!(id));
    }
}

fn renames_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    widgets::dim_label(
        ui,
        tl!(
            "A renamed control keeps its artwork and takes the new name, moving and changing type as the file says. Controls at the same place are paired; pick an added and a removed one and >> to pair others."
        ),
    );
    ui.add_space(8.0);
    let mut pairs = pairs(d);
    let added: Vec<(String, String, String)> =
        unpaired(d, "added", |n| pairs.iter().any(|p| p.1 == n)).into_iter().map(|(n, h)| (n.clone(), h, n)).collect();
    let removed: Vec<(String, String, String)> =
        unpaired(d, "removed", |n| pairs.iter().any(|p| p.0 == n)).into_iter().map(|(n, h)| (n.clone(), h, n)).collect();
    let renamed: Vec<(String, String, String)> = pairs
        .iter()
        .map(|(from, to)| (format!("{from} => {to}"), format!("{}\n{}", detail(d, "removed", from), detail(d, "added", to)), from.clone()))
        .collect();
    ui.columns(3, |cols| {
        if let [a, r, n] = cols {
            column(a, d, &t, "Items Added", "__added", &added);
            column(r, d, &t, "Items Removed", "__removed", &removed);
            column(n, d, &t, "Items Renamed", "__pair", &renamed);
        }
    });
    // The selections may be stale (an item just moved): only those still listed count.
    let has = |list: &[(String, String, String)], field: &str| Some(d.str(field)).filter(|s| list.iter().any(|i| i.2 == *s));
    let (to, from, pair) = (has(&added, "__added"), has(&removed, "__removed"), has(&renamed, "__pair"));
    ui.add_space(8.0);
    let (mut back, mut forth) = (false, false);
    ui.horizontal(|ui| {
        let w = 2.0 * 48.0 + ui.spacing().item_spacing.x;
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        back = ui.add_enabled(pair.is_some(), egui::Button::new("<<").min_size(egui::vec2(48.0, 0.0))).on_hover_text(tl!("Don't rename")).clicked();
        forth = ui
            .add_enabled(to.is_some() && from.is_some(), egui::Button::new(">>").min_size(egui::vec2(48.0, 0.0)))
            .on_hover_text(tl!("Rename the removed item to the added one"))
            .clicked();
    });
    if back && let Some(p) = pair {
        pairs.retain(|(from, _)| *from != p);
    } else if forth && let (Some(from), Some(to)) = (from, to) {
        pairs.push((from, to));
    } else {
        return false;
    }
    d.fields.insert("pairs".into(), json!(pairs.iter().map(|(from, to)| json!({ "from": from, "to": to })).collect::<Vec<_>>()));
    for field in ["__added", "__removed", "__pair"] {
        d.fields.insert(field.into(), json!(""));
    }
    false
}

/// OK: back to Import Positions, which checks the file again with these renames.
fn renames_confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let mut parent = super::parent(d).ok_or("Import Positions is no longer open")?;
    parent.fields.insert("renames".into(), d.fields.get("pairs").cloned().unwrap_or_else(|| json!([])));
    app.ui.dialog = Some(parent);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    const MM: f64 = 72.0 / 25.4;

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::epaint::ClippedShape> {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        out.shapes
    }

    fn shows(shapes: &[egui::epaint::ClippedShape], text: &str) -> bool {
        shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.job.text.contains(text),
            _ => false,
        })
    }

    /// Export from the panel menu's dialog, edit the file, and Import: the dialog shows what will
    /// change before OK, and the document then knows its file.
    #[test]
    fn export_then_import_shows_the_report_and_applies_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100.0 * MM, "height": 60.0 * MM})).unwrap();
        let cutout = app.run("layer.new", json!({"name": "cutout"})).unwrap()["id"].clone();
        app.run("layer.setCurrent", json!({"id": cutout})).unwrap();
        let hole = app.run("shape.ellipse", json!({"x": 17.0 * MM, "y": 37.0 * MM, "width": 6.0 * MM, "height": 6.0 * MM})).unwrap();
        app.run("select.set", json!({"ids": [hole["id"]]})).unwrap();
        app.run("select.save", json!({"name": "Jack 1"})).unwrap();
        let dir = std::env::temp_dir().join(format!("vc-positions-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("panel.csv").to_string_lossy().to_string();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);

        super::open(&mut app, false);
        app.ui.dialog.as_mut().unwrap().fields.insert("path".into(), json!(path));
        frame(&mut app, &ctx);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name,type,x_mm,y_mm,w_mm,h_mm\nJack 1,,-30.000,10.000,,\n");

        std::fs::write(&path, "name,type,x_mm,y_mm,w_mm,h_mm\nJack 1,,-30.000,5.000,,\n").unwrap();
        crate::menus::invoke(&mut app, "positions.import", json!({}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("path"), path, "the document remembers its file");
        frame(&mut app, &ctx);
        let shapes = frame(&mut app, &ctx);
        assert!(shows(&shapes, "Will move (1)") && shows(&shapes, "Jack 1: moved from -30.0, 10.0 to -30.0, 5.0 (up 5.0mm)"));
        let steps = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
        let b = app.session.doc().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(hole["id"].as_u64().unwrap())], false).unwrap();
        assert!((b.center().y / MM - 35.0).abs() < 1e-6);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file that drops Jack 2 and Jack 3 and adds Out 1 where Jack 2 was and Out 2 elsewhere:
    /// Jack 2 => Out 1 is paired by position; Renamed Items… unpairs it with << and pairs Jack 3
    /// => Out 2 with >>, and the import renames Jack 3, keeping its artwork.
    #[test]
    fn renamed_items_pairs_removed_and_added_controls() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100.0 * MM, "height": 60.0 * MM})).unwrap();
        let mut layers = std::collections::HashMap::new();
        for name in ["cutout", "mount"] {
            layers.insert(name, app.run("layer.new", json!({"name": name})).unwrap()["id"].clone());
        }
        let circle = |app: &mut VectorcraftApp, cx: f64, cy: f64, d: f64| {
            let p = json!({"x": (cx - d / 2.0) * MM, "y": (cy - d / 2.0) * MM, "width": d * MM, "height": d * MM});
            app.run("shape.ellipse", p).unwrap()["id"].clone()
        };
        app.run("library.create", json!({"name": "Types"})).unwrap();
        app.run("layer.setCurrent", json!({"id": layers["mount"]})).unwrap();
        let g = circle(&mut app, 150.0, 30.0, 8.0);
        app.run("layer.setProps", json!({"ids": [g], "name": "JackMono"})).unwrap();
        app.run("library.add", json!({"library": "Types", "kind": "graphic", "ids": [g], "name": "JackMono"})).unwrap();
        app.run("edit.clear", json!({"ids": [g]})).unwrap();
        let mut holes = std::collections::HashMap::new();
        for (name, cx, cy) in [("Jack 1", 20.0, 40.0), ("Jack 2", 80.0, 40.0), ("Jack 3", 50.0, 25.0)] {
            app.run("layer.setCurrent", json!({"id": layers["mount"]})).unwrap();
            let m = circle(&mut app, cx, cy, 8.0);
            app.run("layer.setProps", json!({"ids": [m], "name": "JackMono"})).unwrap();
            app.run("layer.setCurrent", json!({"id": layers["cutout"]})).unwrap();
            let c = circle(&mut app, cx, cy, 6.0);
            holes.insert(name, c.clone());
            app.run("select.set", json!({"ids": [m, c]})).unwrap();
            app.run("select.save", json!({"name": name})).unwrap();
        }
        app.run("select.none", json!({})).unwrap();
        let dir = std::env::temp_dir().join(format!("vc-positions-renames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("panel.csv").to_string_lossy().to_string();
        std::fs::write(&path, "name,type,x_mm,y_mm\nJack 1,JackMono,-30.000,10.000\nOut 1,JackMono,30.000,10.000\nOut 2,JackMono,0.000,0.000\n")
            .unwrap();
        super::open(&mut app, true);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("path".into(), json!(path));
        d.fields.insert("library".into(), json!("Types"));

        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 800.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(4);
        let report = |h: &egui_kittest::Harness<VectorcraftApp>| h.state().ui.dialog.as_ref().unwrap().fields["report"].clone();
        assert_eq!(report(&h)["renamed"], json!(["Jack 2 => Out 1"]));
        assert_eq!(
            (&report(&h)["removed"], &report(&h)["added"]),
            (&json!(["Jack 3: JackMono at 0.0, -5.0"]), &json!(["Out 2: JackMono at 0.0, 0.0"]))
        );
        let _ = h.get_by_label("Jack 2 => Out 1");
        h.get_by_label("Renamed Items…").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().kind, super::RENAMES);
        let pairs = |h: &egui_kittest::Harness<VectorcraftApp>| h.state().ui.dialog.as_ref().unwrap().fields["pairs"].clone();
        for label in ["Out 2", "Jack 3", ">>"] {
            h.get_by_label(label).click();
            h.run_steps(2);
        }
        assert_eq!(pairs(&h), json!([{"from": "Jack 2", "to": "Out 1"}, {"from": "Jack 3", "to": "Out 2"}]));
        for label in ["Jack 2 => Out 1", "<<"] {
            h.get_by_label(label).click();
            h.run_steps(2);
        }
        assert_eq!(pairs(&h), json!([{"from": "Jack 3", "to": "Out 2"}]));
        let _ = (h.get_by_label("Out 1"), h.get_by_label("Jack 2"));
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().kind, super::IMPORT, "OK goes back to the import");
        assert_eq!(report(&h)["renamed"], json!(["Jack 3 => Out 2"]));
        assert_eq!(
            (&report(&h)["removed"], &report(&h)["added"]),
            (&json!(["Jack 2: JackMono at 30.0, 10.0"]), &json!(["Out 1: JackMono at 30.0, 10.0"]))
        );
        h.get_by_label("Import").click();
        h.run_steps(2);
        let app = h.into_state();
        assert!(app.ui.dialog.is_none());
        let saved = &app.session.doc().unwrap().doc.saved_selections;
        let set = |name: &str| saved.iter().find(|x| x.name == name).map(|x| x.objects.clone());
        assert_eq!((set("Jack 2"), set("Jack 3")), (None, None));
        let hole = vectorcraft_doc::NodeId(holes["Jack 3"].as_u64().unwrap());
        assert!(set("Out 2").unwrap().contains(&hole), "Out 2 is Jack 3's artwork, renamed and moved");
        assert!(!set("Out 1").unwrap().contains(&vectorcraft_doc::NodeId(holes["Jack 2"].as_u64().unwrap())), "Out 1 is new");
        let b = app.session.doc().unwrap().doc.bounds_of(&[hole], false).unwrap();
        assert!((b.center().x / MM - 50.0).abs() < 1e-6 && (b.center().y / MM - 30.0).abs() < 1e-6, "{b:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// With `CRAFT_UI_POSITIONS_SNAPSHOTS` set to a folder, renders both dialogs there (PNG) on a
    /// panel whose import moves, retypes, adds, removes and renames controls, for a look at them,
    /// and Renamed Items over the import.
    #[test]
    fn snapshots_of_the_dialogs() {
        let Some(directory) = std::env::var_os("CRAFT_UI_POSITIONS_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&directory).unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100.0 * MM, "height": 60.0 * MM})).unwrap();
        let mut layers = std::collections::HashMap::new();
        for name in ["cutout", "mount", "pcb"] {
            layers.insert(name, app.run("layer.new", json!({"name": name})).unwrap()["id"].clone());
        }
        let shape = |app: &mut VectorcraftApp, cmd: &str, cx: f64, cy: f64, w: f64, h: f64| {
            let p = json!({"x": (cx - w / 2.0) * MM, "y": (cy - h / 2.0) * MM, "width": w * MM, "height": h * MM});
            app.run(cmd, p).unwrap()["id"].clone()
        };
        app.run("library.create", json!({"name": "Types"})).unwrap();
        for kind in ["JackMono", "JackStereo", "Pot16mm", "Pot9mm"] {
            app.run("layer.setCurrent", json!({"id": layers["mount"]})).unwrap();
            let g = shape(&mut app, "shape.ellipse", 150.0, 30.0, 8.0, 8.0);
            app.run("layer.setProps", json!({"ids": [g], "name": kind})).unwrap();
            app.run("library.add", json!({"library": "Types", "kind": "graphic", "ids": [g], "name": kind})).unwrap();
            app.run("edit.clear", json!({"ids": [g]})).unwrap();
        }
        for (name, kind, cx, cy) in [("Jack 1", "JackMono", 20.0, 40.0), ("Knob 1", "Pot16mm", 50.0, 25.0), ("Jack 2", "JackStereo", 80.0, 40.0)] {
            app.run("layer.setCurrent", json!({"id": layers["mount"]})).unwrap();
            let m = shape(&mut app, "shape.ellipse", cx, cy, 8.0, 8.0);
            app.run("layer.setProps", json!({"ids": [m], "name": kind})).unwrap();
            app.run("layer.setCurrent", json!({"id": layers["cutout"]})).unwrap();
            let c = shape(&mut app, "shape.ellipse", cx, cy, 6.0, 6.0);
            app.run("select.set", json!({"ids": [m, c]})).unwrap();
            app.run("select.save", json!({"name": name})).unwrap();
        }
        app.run("layer.setCurrent", json!({"id": layers["pcb"]})).unwrap();
        shape(&mut app, "shape.rectangle", 50.0, 30.0, 96.0, 56.0);
        app.run("select.none", json!({})).unwrap();
        let path = directory.join("panel-positions.csv").to_string_lossy().to_string();
        std::fs::write(
            &path,
            "name,type,x_mm,y_mm,w_mm,h_mm\nPCB outline,Edge.Cuts,0.000,0.000,96.000,56.000\nButton 1,Button6mm,10.000,-20.000,,\n\
             Jack 1,JackMono,-30.000,5.000,,\nJack 3,JackStereo,30.000,10.000,,\nKnob 1,Pot9mm,2.000,-5.000,,\n",
        )
        .unwrap();
        for (import, renames, file) in
            [(false, false, "positions-export.png"), (true, false, "positions-import.png"), (true, true, "positions-renames.png")]
        {
            super::open(&mut app, import);
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("path".into(), json!(path));
            d.fields.insert("library".into(), json!("Types"));
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).wgpu().build_ui_state(
                |ui, app: &mut VectorcraftApp| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                },
                app,
            );
            h.input_mut().max_texture_side = Some(8192);
            h.run_steps(6);
            if renames {
                h.get_by_label("Renamed Items…").click();
                h.run_steps(6);
            }
            h.render().unwrap().save(directory.join(file)).unwrap();
            app = h.into_state();
            app.ui.dialog = None;
        }
    }
}
