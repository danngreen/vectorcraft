//! Saved Selections panel: the document's saved selections (Select → Save Selection…), every one,
//! as rows with their name and number of objects. Clicking a row selects all of its objects, those
//! hidden or locked or on hidden or locked layers too (`select.recall {all: true}`), so objects of
//! one thing drawn on several layers move, transform and delete together; Shift- or Cmd/Ctrl-click
//! adds them to the selection, or takes them out when all are selected. Double-click a name to
//! rename it. The buttons save the selection as a new saved selection (named at once), add the
//! selected objects to the chosen row's or remove them from it, and delete the chosen row. The
//! flyout holds Save Selection…, Edit Selection… and Click Selects Saved Selections
//! (`select.savedClick`: a Selection tool click on an object selects the saved selection holding it).

use egui::{Sense, Ui, pos2, vec2};
use serde_json::json;

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

pub const ID: &str = "savedSelections";

/// The row last clicked (by name): the one the buttons act on.
const CHOSEN: &str = "ss-chosen";
/// The row being renamed (by name).
const EDITING: &str = "ss-edit";

/// A row: its name, how many of its objects are still in the document, and whether all of those
/// are selected.
struct Row {
    name: String,
    count: usize,
    selected: bool,
}

fn rows(app: &VectorcraftApp) -> Option<Vec<Row>> {
    let st = app.session.active()?;
    let saved = &st.doc.saved_selections;
    // Every saved selection's objects found in one walk of the tree.
    let live = st.doc.index_paths(saved.iter().flat_map(|s| s.objects.iter().copied()));
    Some(
        saved
            .iter()
            .map(|s| {
                let objects: Vec<_> = s.objects.iter().filter(|id| live.contains_key(id)).collect();
                let selected = !objects.is_empty() && objects.iter().all(|id| st.selection.contains(**id));
                Row { name: s.name.clone(), count: objects.len(), selected }
            })
            .collect(),
    )
}

/// Select saved selection `name`'s objects, hidden and locked ones too: `mode` set, add or remove.
fn recall(app: &mut VectorcraftApp, name: &str, mode: &str) {
    app.run("select.recall", json!({ "name": name, "all": true, "mode": mode })).ok();
}

/// Save the selection as a new saved selection and start renaming it.
fn save_new(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if let Ok(r) = app.run("select.save", json!({ "name": "" }))
        && let Some(name) = r["name"].as_str()
    {
        set_pstate(ctx, CHOSEN, Some(name.to_string()));
        set_pstate(ctx, EDITING, Some(name.to_string()));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(rows) = rows(app) else {
        super::empty_state(ui, "square-dashed", tl!("No document"), tl!("Open a document to see its saved selections."));
        return;
    };
    let something_selected = super::selection_len(app) > 0;
    let chosen: Option<String> = pstate::<Option<String>>(ui.ctx(), CHOSEN).filter(|c| rows.iter().any(|r| r.name == *c));
    if rows.is_empty() {
        super::empty_state(ui, "square-dashed", tl!("No saved selections"), tl!("Save a selection to list it here."));
    } else {
        let editing: Option<String> = pstate(ui.ctx(), EDITING);
        widgets::list_box(ui, |ui| {
            ui.set_min_height(110.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().id_salt("ss-scroll").max_height(320.0).show(ui, |ui| {
                for (i, row) in rows.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    if chosen.as_deref() == Some(row.name.as_str()) {
                        ui.painter().rect_filled(r, 0.0, t.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover);
                    }
                    // A dot marks a saved selection that is all selected.
                    if row.selected {
                        ui.painter().circle_filled(pos2(r.left() + 10.0, r.center().y), 3.0, t.accent);
                    }
                    let name_rect = egui::Rect::from_min_max(pos2(r.left() + 20.0, r.top() + 2.0), pos2(r.right() - 40.0, r.bottom() - 2.0));
                    if editing.as_deref() == Some(row.name.as_str()) {
                        let id = ui.id().with(("ss-name", i));
                        let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| row.name.clone());
                        let te = ui.put(name_rect, egui::TextEdit::singleline(&mut buf).id(id).font(egui::FontId::proportional(12.0)));
                        if !te.has_focus() && !te.lost_focus() {
                            te.request_focus();
                        }
                        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
                        if te.lost_focus() {
                            set_pstate::<Option<String>>(ui.ctx(), EDITING, None);
                            ui.data_mut(|d| d.remove::<String>(id));
                            let new = buf.trim();
                            if !ui.input(|i| i.key_pressed(egui::Key::Escape))
                                && !new.is_empty()
                                && new != row.name
                                && app.run("select.editSaved", json!({ "name": row.name, "newName": new })).is_ok()
                            {
                                set_pstate(ui.ctx(), CHOSEN, Some(new.to_string()));
                            }
                        }
                    } else {
                        // A saved selection's name is the user's: shown as it is.
                        ui.painter().text(name_rect.left_center(), egui::Align2::LEFT_CENTER, &row.name, egui::FontId::proportional(12.5), t.text);
                    }
                    ui.painter().text(
                        pos2(r.right() - 8.0, r.center().y),
                        egui::Align2::RIGHT_CENTER,
                        row.count.to_string(),
                        egui::FontId::proportional(11.5),
                        t.text_dim,
                    );
                    if resp.double_clicked() {
                        set_pstate(ui.ctx(), EDITING, Some(row.name.clone()));
                    } else if resp.clicked() {
                        set_pstate(ui.ctx(), CHOSEN, Some(row.name.clone()));
                        let more = ui.input(|inp| inp.modifiers.shift || inp.modifiers.command);
                        let mode = match (more, row.selected) {
                            (false, _) => "set",
                            (true, false) => "add",
                            (true, true) => "remove",
                        };
                        recall(app, &row.name, mode);
                    }
                }
            });
        });
    }
    widgets::bottom_bar(ui, |ui| {
        ui.add_space((ui.available_width() - 4.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "plus", tl!("Add Selected Objects"), false, something_selected && chosen.is_some(), 24.0).clicked()
            && let Some(name) = &chosen
        {
            app.run("select.editSaved", json!({ "name": name, "add": true })).ok();
        }
        if widgets::icon_button_enabled(ui, "minus", tl!("Remove Selected Objects"), false, something_selected && chosen.is_some(), 24.0).clicked()
            && let Some(name) = &chosen
        {
            app.run("select.editSaved", json!({ "name": name, "remove": true })).ok();
        }
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("Save Selection…"), false, something_selected, 24.0).clicked() {
            save_new(app, ui.ctx());
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Selection"), false, chosen.is_some(), 24.0).clicked()
            && let Some(name) = &chosen
            && app.run("select.editSaved", json!({ "name": name, "delete": true })).is_ok()
        {
            set_pstate::<Option<String>>(ui.ctx(), CHOSEN, None);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let has_doc = app.session.active().is_some();
    let saved = app.session.active().is_some_and(|d| !d.doc.saved_selections.is_empty());
    if menu_item(ui, tl!("Save Selection…"), has_doc && super::selection_len(app) > 0, false) {
        crate::menus::invoke(app, "select.save", json!({}));
    }
    if menu_item(ui, tl!("Edit Selection…"), saved, false) {
        crate::menus::invoke(app, "select.editSaved", json!({}));
    }
    ui.separator();
    let on = app.session.prefs.click_selects_saved_selections;
    if menu_item(ui, tl!("Click Selects Saved Selections"), true, on) {
        app.run("select.savedClick", json!({ "on": !on })).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeId;

    fn id_of(v: &serde_json::Value) -> NodeId {
        NodeId(v["id"].as_u64().unwrap())
    }

    /// A rectangle on the first layer and one on a hidden layer, saved as `Knob`.
    fn app() -> (VectorcraftApp, [NodeId; 2]) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let s = &mut app.session;
        s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let a = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap());
        let layer = id_of(&s.execute("layer.new", &json!({"name": "Hidden"})).unwrap());
        let b = id_of(&s.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap());
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        s.execute("select.save", &json!({"name": "Knob"})).unwrap();
        s.execute("layer.setProps", &json!({"ids": [layer.0], "visible": false})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        (app, [a, b])
    }

    fn selected(app: &VectorcraftApp) -> Vec<NodeId> {
        let mut v = app.session.active().unwrap().selection.objects.clone();
        v.sort();
        v
    }

    /// One frame of the panel with `events`, 0.2 s after the one before (a press and its release
    /// are a click, two clicks are never a double-click); the shapes it painted.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
        frame_with(app, ctx, events, egui::Modifiers::default())
    }

    /// [`frame`] with `modifiers` held.
    fn frame_with(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Vec<egui::epaint::ClippedShape> {
        let time = Some(ctx.input(|i| i.time) + 0.2);
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let mut out = ctx.run_ui(egui::RawInput { events, time, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2, modifiers: egui::Modifiers) {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        for e in [egui::Event::PointerMoved(at), button(true), button(false)] {
            frame_with(app, ctx, vec![e], modifiers);
        }
    }

    fn text_at(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Pos2> {
        shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + vec2(4.0, 4.0)),
            _ => None,
        })
    }

    /// A click on a row selects every object of it, the one on a hidden layer too; Shift-click on it
    /// all selected takes it out again.
    #[test]
    fn clicking_a_row_selects_all_its_objects_hidden_ones_too() {
        let (mut app, [a, b]) = app();
        let ctx = egui::Context::default();
        let at = text_at(&frame(&mut app, &ctx, vec![]), "Knob").expect("a row for Knob");
        click(&mut app, &ctx, at, egui::Modifiers::default());
        assert_eq!(selected(&app), vec![a, b]);
        click(&mut app, &ctx, at, egui::Modifiers::SHIFT);
        assert!(selected(&app).is_empty());
    }

    /// The row shows how many of its objects are left.
    #[test]
    fn a_row_counts_its_objects() {
        let (mut app, [a, _]) = app();
        let ctx = egui::Context::default();
        assert!(text_at(&frame(&mut app, &ctx, vec![]), "2").is_some());
        app.session.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
        assert!(text_at(&frame(&mut app, &ctx, vec![]), "1").is_some());
    }

    /// Without saved selections the panel says how to make one.
    #[test]
    fn an_empty_list_says_how_to_save_one() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        assert!(text_at(&frame(&mut app, &ctx, vec![]), "Save a selection to list it here.").is_some());
    }
}
