//! File › Export › Export Faceplate…: the artwork layer as a KiCad footprint (`faceplate.export`):
//! the footprint file, the artwork's and the faceplate outline's layers, and the resolution the
//! artwork is read at.

use std::path::Path;

use serde_json::{Value, json};

use super::{Dialog, DialogResult, DialogSpec, form};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const KIND: &str = "faceplateExport";

/// The fields, as the command names them, with their labels.
const SETTINGS: [(&str, &str); 3] = [("layer", "Artwork Layer"), ("faceplateLayer", "Faceplate Layer"), ("ppi", "Resolution (ppi)")];

/// The fields the dialog opens with: the command's defaults for the active document.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    let mut fields = vectorcraft_engine::cmd::faceplate::defaults(&app.session);
    // Typed as text, like the layers.
    fields["ppi"] = json!(fields["ppi"].as_f64().map_or_else(String::new, |v| v.to_string()));
    fields
}

/// The command params of the dialog's fields.
fn params(d: &Dialog) -> Value {
    let mut p = json!({ "path": d.str("path"), "layer": d.str("layer"), "faceplateLayer": d.str("faceplateLayer") });
    if let Ok(ppi) = d.str("ppi").trim().parse::<f64>() {
        p["ppi"] = json!(ppi);
    }
    p
}

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Export Faceplate").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 440.0,
    max_width: Some(560.0),
    ok_enabled: Some(|_, d| !d.str("path").trim().is_empty() && d.str("ppi").trim().parse::<f64>().is_ok()),
    ..DialogSpec::FORM
};

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    widgets::dim_label(
        ui,
        tl!(
            "Writes the artwork layer as a KiCad footprint for a faceplate PCB: its dark areas as silkscreen, and the faceplate layer's shape as the board's edge."
        ),
    );
    ui.add_space(8.0);
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
    ui.add_space(6.0);
    egui::Grid::new("faceplate-settings").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        for (key, label) in SETTINGS {
            form::field(ui, d, key, label);
        }
    });
    false
}

/// Pick where the footprint goes.
fn choose(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let current = d.str("path");
    let doc_dir =
        app.session.active().and_then(|st| st.path.as_deref()).and_then(|p| Path::new(p).parent()).map(|p| p.to_string_lossy().into_owned());
    let folder =
        Path::new(&current).parent().map(|p| p.to_string_lossy().into_owned()).filter(|f| !f.is_empty() && Path::new(f).is_dir()).or(doc_dir);
    let name = Path::new(&current).file_name().map_or_else(|| "faceplate.kicad_mod".to_string(), |n| n.to_string_lossy().into_owned());
    let pick = crate::FilePick { name, folder, filters: vec![crate::FilePick::filter("KiCad Footprint", &["kicad_mod"])] };
    if let Some(path) = crate::picks::save(app, &pick) {
        d.fields.insert("path".into(), json!(path));
    }
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let r = super::run_and_close(app, "faceplate.export", params(d))?;
    let file = Path::new(&d.str("path")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let count = r["polygons"].as_u64().unwrap_or(0).to_string();
    let mut message = crate::i18n::fmt(tl!("Exported {count} shapes of artwork to {file}"), &[("count", &count), ("file", &file)]);
    // What is missing from the footprint (no outline, no artwork) is said with it.
    for w in r["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        message.push_str("; ");
        message.push_str(w);
    }
    app.status(message);
    Ok(r)
}
