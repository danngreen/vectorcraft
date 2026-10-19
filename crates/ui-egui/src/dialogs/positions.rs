//! Saved Selections panel → Export Positions… / Import Positions…: the CSV file, the layers and
//! the Library (`positions.settings`: the document's, else the preferences'), and, for an import,
//! the report of what it will change (`positions.check`) before OK applies it in one undo step.

use std::path::Path;

use serde_json::{Value, json};

use super::{Dialog, DialogResult, DialogSpec, form};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const EXPORT: &str = "positionsExport";
pub const IMPORT: &str = "positionsImport";

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
        if !d.str("path").is_empty() && ui.button(tl!("Check Again")).clicked() {
            // The file may have changed on disk since: read it again.
            d.fields.remove("checked");
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
    let pick = crate::FilePick { name, folder, filters: vec![("CSV", &["csv"])] };
    let path = if import(d) { crate::picks::open(app, &pick) } else { crate::picks::save(app, &pick) };
    if let Some(path) = path {
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

#[cfg(test)]
mod tests {
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

    /// With `CRAFT_UI_POSITIONS_SNAPSHOTS` set to a folder, renders both dialogs there (PNG) on a
    /// panel whose import moves, retypes, adds and removes controls, for a look at them.
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
             Jack 1,JackMono,-30.000,5.000,,\nJack 3,JackStereo,15.000,10.000,,\nKnob 1,Pot9mm,2.000,-5.000,,\n",
        )
        .unwrap();
        for (import, file) in [(false, "positions-export.png"), (true, "positions-import.png")] {
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
            h.render().unwrap().save(directory.join(file)).unwrap();
            app = h.into_state();
            app.ui.dialog = None;
        }
    }
}
