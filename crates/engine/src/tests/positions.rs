//! Export/Import Positions: a panel's saved selections as a positions CSV and back.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::*;

const MM: f64 = 72.0 / 25.4;

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids(v: &[NodeId]) -> Vec<u64> {
    v.iter().map(|i| i.0).collect()
}

struct Panel {
    s: Session,
    layers: std::collections::HashMap<&'static str, NodeId>,
}

impl Panel {
    /// A 100 × 60 mm panel with layers cutout, mount, mockup and silk, and a Library `Types` of
    /// mount groups.
    fn new() -> Self {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 100.0 * MM, "height": 60.0 * MM})).unwrap();
        let mut layers = std::collections::HashMap::new();
        for name in ["cutout", "mount", "mockup", "silk", "pcb"] {
            layers.insert(name, id(&s.execute("layer.new", &json!({"name": name})).unwrap()));
        }
        let mut p = Panel { s, layers };
        p.s.execute("library.create", &json!({"name": "Types"})).unwrap();
        for (i, kind) in ["JackMono", "JackStereo", "Pot16mm", "Pot9mm", "Button6mm"].into_iter().enumerate() {
            let g = p.mount_group(kind, 150.0 + 20.0 * i as f64, 30.0);
            p.s.execute("library.add", &json!({"library": "Types", "kind": "graphic", "ids": [g.0], "name": kind})).unwrap();
            p.s.execute("edit.clear", &json!({"ids": [g.0]})).unwrap();
        }
        p.s.execute("prefs.set", &json!({"key": "positionsLibrary", "value": "Types"})).unwrap();
        p
    }

    fn on(&mut self, layer: &str) {
        self.s.execute("layer.setCurrent", &json!({"id": self.layers[layer].0})).unwrap();
    }

    fn circle(&mut self, cx: f64, cy: f64, d: f64) -> NodeId {
        let p = json!({"x": (cx - d / 2.0) * MM, "y": (cy - d / 2.0) * MM, "width": d * MM, "height": d * MM});
        id(&self.s.execute("shape.ellipse", &p).unwrap())
    }

    fn rect(&mut self, cx: f64, cy: f64, w: f64, h: f64) -> NodeId {
        let p = json!({"x": (cx - w / 2.0) * MM, "y": (cy - h / 2.0) * MM, "width": w * MM, "height": h * MM});
        id(&self.s.execute("shape.rectangle", &p).unwrap())
    }

    fn mount_group(&mut self, kind: &str, cx: f64, cy: f64) -> NodeId {
        self.on("mount");
        let size = if kind.starts_with("Pot16") { 16.0 } else { 6.0 };
        let parts = [self.circle(cx, cy, size), self.rect(cx, cy + size / 2.0 + 1.0, size, 2.0)];
        let g = id(&self.s.execute("object.group", &json!({"ids": ids(&parts)})).unwrap());
        self.s.execute("layer.setProps", &json!({"ids": [g.0], "name": kind})).unwrap();
        g
    }

    /// A control: its mount group, a cutout, a mockup knob and a silkscreen label, saved as `name`.
    fn control(&mut self, name: &str, kind: &str, cx: f64, cy: f64, hole: f64) -> Vec<NodeId> {
        let m = self.mount_group(kind, cx, cy);
        self.on("cutout");
        let c = self.circle(cx, cy, hole);
        self.on("mockup");
        let k = self.circle(cx, cy, hole + 4.0);
        self.on("silk");
        let t = self.rect(cx, cy - hole / 2.0 - 3.0, 10.0, 2.0);
        let members = vec![m, c, k, t];
        self.s.execute("select.set", &json!({"ids": ids(&members)})).unwrap();
        self.s.execute("select.save", &json!({"name": name})).unwrap();
        members
    }

    /// Three controls, the mount layer locked and the mockup layer hidden, as a panel in progress.
    fn with_controls() -> Self {
        let mut p = Panel::new();
        p.control("Jack 1", "JackMono", 20.0, 40.0, 6.0);
        p.control("Knob 1", "Pot16mm", 50.0, 25.0, 7.0);
        p.control("Jack 2", "JackStereo", 80.0, 40.0, 6.0);
        p.s.execute("layer.setProps", &json!({"ids": [p.layers["mount"].0], "locked": true})).unwrap();
        p.s.execute("layer.setProps", &json!({"ids": [p.layers["mockup"].0], "visible": false})).unwrap();
        p.s.execute("select.none", &json!({})).unwrap();
        p
    }

    fn csv(&mut self) -> String {
        self.s.execute("positions.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string()
    }

    fn import(&mut self, csv: &str) -> Value {
        self.s.execute("positions.import", &json!({"csv": csv})).unwrap()
    }

    fn state(&self) -> (Vec<std::sync::Arc<vectorcraft_doc::Node>>, Vec<vectorcraft_doc::SavedSelection>) {
        let doc = &self.s.doc().unwrap().doc;
        (doc.layers.clone(), doc.saved_selections.to_vec())
    }

    fn set(&self, name: &str) -> Vec<NodeId> {
        let doc = &self.s.doc().unwrap().doc;
        doc.saved_selections.iter().find(|x| x.name == name).map(|x| x.objects.clone()).unwrap_or_default()
    }

    fn center_mm(&self, id: NodeId) -> (f64, f64) {
        let doc = &self.s.doc().unwrap().doc;
        let c = doc.bounds_of(&[id], false).unwrap().center();
        let o = doc.artboards[0].rect.center();
        (((c.x - o.x) / MM * 1000.0).round() / 1000.0, ((c.y - o.y) / MM * 1000.0).round() / 1000.0)
    }
}

const EXPORTED: &str = "name,type,x_mm,y_mm,w_mm,h_mm\n\
    Jack 1,JackMono,-30.000,10.000,,\n\
    Jack 2,JackStereo,30.000,10.000,,\n\
    Knob 1,Pot16mm,0.000,-5.000,,\n";

#[test]
fn export_writes_each_saved_selection_at_its_cutout_from_the_panel_centre() {
    let mut p = Panel::with_controls();
    assert_eq!(p.csv(), EXPORTED);
}

#[test]
fn the_pcb_outline_is_the_largest_square_rectangle_on_the_pcb_layer() {
    let mut p = Panel::with_controls();
    p.on("pcb");
    p.rect(50.0, 30.0, 96.0, 56.0);
    p.rect(20.0, 20.0, 10.0, 10.0);
    // A rotated rectangle larger than both can't be an outline.
    let r = p.rect(50.0, 30.0, 99.0, 59.0);
    p.s.execute("object.rotate", &json!({"ids": [r.0], "angle": 10})).unwrap();
    let out = p.s.execute("positions.export", &json!({})).unwrap();
    let csv = out["csv"].as_str().unwrap();
    assert!(csv.starts_with("name,type,x_mm,y_mm,w_mm,h_mm\nPCB outline,Edge.Cuts,0.000,0.000,96.000,56.000\nJack 1,"), "{csv}");
    assert!(out["warnings"][0].as_str().unwrap().contains("1 rotated or round-cornered rectangle"), "{out}");
}

#[test]
fn import_moves_whole_controls_hidden_and_locked_members_too_in_one_undo_step() {
    let mut p = Panel::with_controls();
    let before = p.state();
    let jack = p.set("Jack 1");
    let r = p.import(&EXPORTED.replace("Jack 1,JackMono,-30.000,10.000", "Jack 1,JackMono,-30.000,5.000"));
    assert_eq!(r["moved"], json!(["Jack 1: JackMono moved from -30.0, 10.0 to -30.0, 5.0 (up 5.0mm)"]));
    assert_eq!(r["changes"], json!(1));
    // Every member moved up 5 mm: the mount (locked layer), cutout, mockup (hidden layer), silk.
    assert_eq!(p.center_mm(jack[1]), (-30.0, 5.0));
    assert_eq!(p.center_mm(jack[2]), (-30.0, 5.0));
    assert_eq!(p.center_mm(jack[3]).1, -1.0);
    assert_eq!(p.center_mm(jack[0]).1, 6.0);
    // The layers are as they were, nothing is selected, and Undo brings everything back.
    let doc = &p.s.doc().unwrap().doc;
    assert!(doc.node(p.layers["mount"]).unwrap().locked && !doc.node(p.layers["mockup"]).unwrap().visible);
    assert!(p.s.doc().unwrap().selection.objects.is_empty());
    p.s.execute("edit.undo", &json!({})).unwrap();
    assert!(p.state() == before, "one undo step restores the panel");
    assert_eq!(p.csv(), EXPORTED);
}

#[test]
fn import_retypes_removes_and_adds_controls_from_the_library() {
    let mut p = Panel::with_controls();
    let before = p.state();
    let csv = "name,type,x_mm,y_mm\n\
        Jack 1,JackMono,-30.000,10.000\n\
        Knob 1,Pot9mm,2.000,-5.000\n\
        Button 1,Button6mm,10.000,-20.000\n\
        Mystery 1,NoSuchType,-10.000,-20.000\n";
    let check = p.s.execute("positions.check", &json!({"csv": csv})).unwrap();
    assert_eq!(p.state(), before, "a check changes nothing");
    let r = p.import(csv);
    assert_eq!(r, check, "the import does what the check said");
    assert_eq!(r["removed"], json!(["Jack 2: JackStereo at 30.0, 10.0"]));
    assert_eq!(r["retyped"], json!(["Knob 1: Pot16mm => Pot9mm"]));
    assert_eq!(r["moved"], json!(["Knob 1: Pot16mm moved from 0.0, -5.0 to 2.0, -5.0 (right 2.0mm)"]));
    assert_eq!(r["added"], json!(["Button 1: Button6mm at 10.0, -20.0", "Mystery 1: red marker at -10.0, -20.0"]));
    assert_eq!(r["errors"], json!(["Mystery 1: type NoSuchType isn't in the Library Types; a red marker goes there instead"]));
    assert_eq!(
        p.csv(),
        "name,type,x_mm,y_mm,w_mm,h_mm\n\
         Button 1,Button6mm,10.000,-20.000,,\n\
         Jack 1,JackMono,-30.000,10.000,,\n\
         Knob 1,Pot9mm,2.000,-5.000,,\n\
         Mystery 1,,-10.000,-20.000,,\n"
    );
    // The retyped knob keeps its other members; the marker is a red circle on the cutout layer.
    assert_eq!(p.set("Knob 1").len(), 4);
    let marker = p.set("Mystery 1")[0];
    let doc = &p.s.doc().unwrap().doc;
    assert_eq!(doc.layer_of(marker), Some(p.layers["cutout"]));
    assert_eq!(doc.node(marker).unwrap().appearance.fill_paint().label(), "#ff0000");
    assert!(p.s.execute("document.find", &json!({"name": "Jack 2"})).unwrap()["matches"].as_array().unwrap().is_empty());
    p.s.execute("edit.undo", &json!({})).unwrap();
    assert!(p.state() == before, "one undo step restores the panel");
}

#[test]
fn a_blank_type_keeps_the_control_s_type_and_a_missing_library_is_an_error() {
    let mut p = Panel::with_controls();
    let r = p.import(&EXPORTED.replace("Jack 1,JackMono,", "Jack 1,,"));
    assert_eq!(r["changes"], json!(0), "{r}");
    let r = p.s.execute("positions.check", &json!({"csv": EXPORTED.replace("Pot16mm", "Pot9mm"), "library": "Nope"})).unwrap();
    assert_eq!(r["errors"], json!(["there's no Library named Nope, so no type can change and new controls are red markers"]));
    assert_eq!(r["changes"], json!(0));
}

#[test]
fn a_different_outline_in_the_file_is_reported_and_left_alone() {
    let mut p = Panel::with_controls();
    p.on("pcb");
    p.rect(50.0, 30.0, 96.0, 56.0);
    let csv = p.csv().replace("96.000", "90.000");
    let r = p.import(&csv);
    assert_eq!(r["changes"], json!(0));
    let w = r["warnings"].as_array().unwrap();
    assert!(
        w.iter().any(|w| w.as_str().unwrap().starts_with("the file's PCB outline (90.0 x 56.0mm at 0.0, 0.0) isn't the pcb layer's (96.0 x 56.0mm")),
        "{r}"
    );
}

#[test]
fn an_object_named_like_a_row_moves_when_no_saved_selection_has_the_name() {
    let mut p = Panel::with_controls();
    p.on("cutout");
    let led = p.circle(10.0, 10.0, 3.0);
    p.s.execute("layer.setProps", &json!({"ids": [led.0], "name": "LED 1"})).unwrap();
    let r = p.import(&format!("{EXPORTED}LED 1,,-35.000,-15.000,,\n"));
    assert_eq!(r["moved"], json!(["LED 1: moved from -40.0, -20.0 to -35.0, -15.0 (down 5.0mm, right 5.0mm)"]));
    assert!(r["warnings"][0].as_str().unwrap().starts_with("LED 1: no saved selection by that name"), "{r}");
    assert_eq!(p.center_mm(led), (-35.0, -15.0));
}

#[test]
fn bad_rows_are_errors_and_a_bad_header_refuses_the_file() {
    let mut p = Panel::with_controls();
    let e = p.s.execute("positions.check", &json!({"csv": "id,x,y\n"})).unwrap_err().to_string();
    assert!(e.contains("the first line must be name,type,x_mm,y_mm,w_mm,h_mm"), "{e}");
    let r =
        p.s.execute("positions.check", &json!({"csv": format!("{EXPORTED}Jack 9,JackMono,left,3\nJack 1,JackMono,1,1,,\n,JackMono,1,1\n")})).unwrap();
    assert_eq!(
        r["errors"],
        json!(["line 5: Jack 9: x_mm and y_mm must be numbers", "line 6: Jack 1 appears twice; the first row is used", "line 7: no name"])
    );
    assert!(p.s.execute("positions.import", &json!({})).is_err(), "no file and none remembered");
}

#[test]
fn settings_come_from_the_params_else_the_document_else_the_preferences() {
    let mut p = Panel::with_controls();
    let dir = std::env::temp_dir().join(format!("vc-positions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("panel.csv").to_string_lossy().to_string();
    let r = p.s.execute("positions.settings", &json!({})).unwrap();
    assert_eq!((r["anchorLayer"].as_str(), r["library"].as_str(), &r["path"]), (Some("cutout"), Some("Types"), &Value::Null));
    // An export to a file keeps the file and the settings unlike the preferences' with the document.
    p.s.execute("positions.export", &json!({"path": path, "typeLayer": "mount", "outlineLayer": "board"})).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXPORTED);
    assert_eq!(p.s.doc().unwrap().doc.positions, json!({"path": path, "outlineLayer": "board"}).as_object().unwrap().clone());
    let r = p.s.execute("positions.settings", &json!({})).unwrap();
    assert_eq!((r["outlineLayer"].as_str(), r["path"].as_str()), (Some("board"), Some(path.as_str())));
    // Import then reads that file without being told.
    std::fs::write(&path, EXPORTED.replace("Jack 2,JackStereo,30.000", "Jack 2,JackStereo,31.000")).unwrap();
    let r = p.s.execute("positions.import", &json!({})).unwrap();
    assert_eq!(r["changes"], json!(1));
    // The preferences still decide what the document doesn't keep.
    p.s.execute("prefs.set", &json!({"key": "positionsAnchorLayer", "value": "holes"})).unwrap();
    assert_eq!(p.s.execute("positions.settings", &json!({})).unwrap()["anchorLayer"], json!("holes"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_control_dropped_where_one_is_added_is_renamed_keeping_its_artwork() {
    let mut p = Panel::with_controls();
    let before = p.state();
    let jack = p.set("Jack 2");
    // Jack 2 is gone and Jack 3 is new at its place, now mono: one control renamed and retyped.
    let csv = EXPORTED.replace("Jack 2,JackStereo,30.000,10.000", "Jack 3,JackMono,30.000,10.000");
    let r = p.import(&csv);
    assert_eq!(r["renamed"], json!(["Jack 2 => Jack 3"]));
    assert_eq!(r["retyped"], json!(["Jack 3: JackStereo => JackMono"]));
    assert_eq!((&r["removed"], &r["added"], &r["changes"]), (&json!([]), &json!([]), &json!(2)), "{r}");
    assert_eq!(
        r["rename"],
        json!({
            "removed": [{"name": "Jack 2", "detail": "JackStereo at 30.0, 10.0"}],
            "added": [{"name": "Jack 3", "detail": "JackMono at 30.0, 10.0"}],
            "pairs": [{"from": "Jack 2", "to": "Jack 3"}],
        })
    );
    // The saved selection has the new name and its cutout, mockup and silk; only the mount is new.
    let set = p.set("Jack 3");
    assert!(p.set("Jack 2").is_empty());
    assert_eq!(set.len(), 4);
    assert!(jack[1..].iter().all(|m| set.contains(m)) && !set.contains(&jack[0]), "{set:?} {jack:?}");
    assert_eq!(p.csv(), csv);
    p.s.execute("edit.undo", &json!({})).unwrap();
    assert!(p.state() == before, "one undo step restores the panel");
}

#[test]
fn renames_given_pair_controls_anywhere_and_none_given_keeps_remove_and_add() {
    let mut p = Panel::with_controls();
    let jack = p.set("Jack 2");
    let csv = EXPORTED.replace("Jack 2,JackStereo,30.000,10.000", "Out 1,JackStereo,35.000,10.000");
    // Not at the same place: offered, not paired.
    let r = p.s.execute("positions.check", &json!({"csv": csv})).unwrap();
    assert_eq!((&r["renamed"], &r["rename"]["pairs"]), (&json!([]), &json!([])));
    assert_eq!((&r["removed"], &r["added"]), (&json!(["Jack 2: JackStereo at 30.0, 10.0"]), &json!(["Out 1: JackStereo at 35.0, 10.0"])));
    // Paired by hand: renamed and moved, nothing removed or added.
    let r = p.s.execute("positions.import", &json!({"csv": csv, "renames": [{"from": "Jack 2", "to": "Out 1"}]})).unwrap();
    assert_eq!(r["renamed"], json!(["Jack 2 => Out 1"]));
    assert_eq!(r["moved"], json!(["Out 1: JackStereo moved from 30.0, 10.0 to 35.0, 10.0 (right 5.0mm)"]));
    assert_eq!((&r["removed"], &r["added"], &r["retyped"]), (&json!([]), &json!([]), &json!([])), "{r}");
    assert_eq!(p.set("Out 1"), jack);
    assert_eq!(p.center_mm(jack[1]), (35.0, 10.0));
    p.s.execute("edit.undo", &json!({})).unwrap();
    // At the same place but unpaired by hand (an empty list): removed and added as before.
    let csv = EXPORTED.replace("Jack 2,JackStereo,30.000,10.000", "Out 1,JackStereo,30.000,10.000");
    let r = p.s.execute("positions.check", &json!({"csv": csv, "renames": []})).unwrap();
    assert_eq!((&r["renamed"], r["removed"].as_array().unwrap().len(), r["added"].as_array().unwrap().len()), (&json!([]), 1, 1));
    // A pair that isn't a removed control and a new row is an error, and left out.
    let r = p.s.execute("positions.check", &json!({"csv": csv, "renames": [{"from": "Jack 1", "to": "Out 1"}]})).unwrap();
    assert_eq!(r["errors"], json!(["Jack 1 => Out 1: Jack 1 isn't a control the file removes, so it isn't renamed"]));
    assert_eq!(r["renamed"], json!([]));
}

#[test]
fn no_rename_is_offered_unless_the_file_both_removes_and_adds() {
    let mut p = Panel::with_controls();
    let r = p.s.execute("positions.check", &json!({"csv": EXPORTED.replace("Jack 2,JackStereo,30.000,10.000,,\n", "")})).unwrap();
    assert_eq!((r["removed"].as_array().unwrap().len(), &r["rename"]), (1, &Value::Null));
    let r = p.s.execute("positions.check", &json!({"csv": format!("{EXPORTED}Jack 9,JackMono,1,1,,\n")})).unwrap();
    assert_eq!((r["added"].as_array().unwrap().len(), &r["rename"]), (1, &Value::Null));
}
