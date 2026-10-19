//! New type starts with the character attributes (font, size, tracking, kerning, scales, …) and
//! paragraph attributes (alignment, indents, spacing, …) last applied to type, not the defaults;
//! its paint still comes from the toolbar.

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Justify, NodeKind, ParaStyle};
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    // Made-up family names stay put without a font that has their glyphs.
    s.prefs.missing_glyph_protection = false;
    s.prefs.placeholder_text = false;
    s
}

fn style(s: &Session, id: &Value) -> CharStyle {
    match &s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.first_style(),
        _ => panic!("not text"),
    }
}

fn made(s: &mut Session, p: Value) -> CharStyle {
    let id = s.execute("text.create", &p).unwrap()["id"].clone();
    style(s, &id)
}

/// Click with the Type tool at (`x`, `y`) → the new type's id.
fn click_type(s: &mut Session, x: f64, y: f64) -> Value {
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
    json!(s.doc().unwrap().selection.objects[0].0)
}

#[test]
fn type_tool_keeps_the_last_font_and_size_across_tools() {
    let mut s = session();
    let first = click_type(&mut s, 100.0, 100.0);
    s.tool_text("Hello", ViewInfo::default()).unwrap();
    // The Character panel ends the typing session, then styles the type being edited.
    s.set_tool_option("commitTyping", &json!(true));
    s.commit_interaction().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": first, "font": "Probe Serif", "style": "Bold", "size": 31, "tracking": 40})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [first], "kerning": 25, "hScale": 90, "underline": true})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let second = click_type(&mut s, 100.0, 300.0);
    assert_ne!(first, second);
    let st = style(&s, &second);
    assert_eq!((st.font_family.as_str(), st.font_style.as_str(), st.size, st.tracking), ("Probe Serif", "Bold", 31.0, 40.0));
    assert_eq!((st.kerning, st.h_scale, st.underline), (Some(25.0), 90.0, true));
    // text.create's own params still win.
    let given = made(&mut s, json!({"x": 10, "y": 500, "text": "a", "font": "Other Sans", "size": 9}));
    assert_eq!((given.font_family.as_str(), given.size, given.tracking), ("Other Sans", 9.0, 40.0));
}

#[test]
fn new_type_takes_the_toolbar_paint_not_the_remembered_one() {
    let mut s = session();
    let t = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap()["id"].clone();
    s.execute("text.setRangeStyle", &json!({"id": t, "size": 20, "fill": "#ff0000", "stroke": "#00ff00", "strokeWidth": 3})).unwrap();
    let st = made(&mut s, json!({"x": 10, "y": 100, "text": "b"}));
    let fresh = made(&mut session(), json!({"x": 10, "y": 100, "text": "b"}));
    assert_eq!(st.size, 20.0);
    assert_eq!((&st.fill, &st.stroke, st.stroke_width), (&fresh.fill, &fresh.stroke, fresh.stroke_width));
}

#[test]
fn sizes_are_remembered_in_document_points() {
    let mut s = session();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Big", "size": 10})).unwrap();
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    // 30 pt as the Character panel shows it: 15 pt in the scaled type's own units.
    s.execute("text.setStyle", &json!({"size": 30})).unwrap();
    assert_eq!(made(&mut s, json!({"x": 10, "y": 300, "text": "b"})).size, 30.0);
}

#[test]
fn fill_alone_is_not_remembered() {
    let mut s = session();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Odd", "font": "Probe Serif", "size": 50})).unwrap();
    // A fill alone doesn't make this type's attributes the ones new type takes.
    s.execute("text.setStyle", &json!({"fill": "#336699"})).unwrap();
    assert_eq!(made(&mut s, json!({"x": 10, "y": 300, "text": "b"})), made(&mut session(), json!({"x": 10, "y": 300, "text": "b"})));
}

fn para(s: &Session, id: &Value) -> ParaStyle {
    match &s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    }
}

#[test]
fn type_tool_keeps_the_last_paragraph_attributes() {
    let mut s = session();
    let first = click_type(&mut s, 100.0, 100.0);
    s.tool_text("Hello", ViewInfo::default()).unwrap();
    s.set_tool_option("commitTyping", &json!(true));
    s.commit_interaction().unwrap();
    // The Paragraph panel's alignment buttons and fields, on the paragraphs being edited.
    s.execute("text.setStyle", &json!({"ids": [first], "start": 0, "end": 5, "justify": "center"})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [first], "start": 0, "end": 5, "leftIndent": 12, "spaceAfter": 4, "hyphenate": true})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let second = click_type(&mut s, 100.0, 300.0);
    let p = para(&s, &second);
    assert_eq!((p.justify, p.left_indent, p.space_after, p.hyphenate), (Justify::Center, 12.0, 4.0, true));
    // The character attributes stay as they were: none were applied.
    assert_eq!(style(&s, &second), style(&s, &first));
    // text.create's `justify` still wins; text.createInPath takes them too.
    let id = s.execute("text.create", &json!({"x": 10, "y": 500, "text": "a", "justify": "right"})).unwrap()["id"].clone();
    assert_eq!((para(&s, &id).justify, para(&s, &id).left_indent), (Justify::Right, 12.0));
    let path = s.execute("shape.rectangle", &json!({"x": 400, "y": 200, "width": 100, "height": 80})).unwrap()["id"].clone();
    let id = s.execute("text.createInPath", &json!({"path": path, "mode": "area", "text": "in"})).unwrap()["id"].clone();
    assert_eq!(para(&s, &id).justify, Justify::Center);
}

#[test]
fn paragraph_changes_leave_the_character_attributes_and_style_link_out() {
    let mut s = session();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Odd", "font": "Probe Serif", "size": 50})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Airy", "attrs": {"space_before": 9}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Airy"})).unwrap();
    s.execute("text.setFormat", &json!({"firstLineIndent": 6})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 300, "text": "b"})).unwrap()["id"].clone();
    let p = para(&s, &id);
    assert_eq!((p.first_line_indent, p.space_before, p.style_name.as_deref()), (6.0, 9.0, None));
    // Only the paragraph attributes were applied: the character attributes are the defaults.
    assert_eq!(style(&s, &id), made(&mut session(), json!({"x": 10, "y": 300, "text": "b"})));
}

#[test]
fn a_cancelled_preview_is_not_remembered_a_committed_one_is() {
    let mut s = session();
    let t = s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hover"})).unwrap()["id"].clone();
    s.begin_interaction("Font").unwrap();
    s.preview("text.setRangeStyle", &json!({"id": t, "font": "Hovered Sans"})).unwrap();
    s.cancel_interaction().unwrap();
    assert_ne!(made(&mut s, json!({"x": 10, "y": 100, "text": "b"})).font_family, "Hovered Sans");
    s.begin_interaction("Font").unwrap();
    s.preview("text.setRangeStyle", &json!({"id": t, "font": "Hovered Sans"})).unwrap();
    s.preview("text.setRangeStyle", &json!({"id": t, "font": "Chosen Sans"})).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(made(&mut s, json!({"x": 10, "y": 150, "text": "c"})).font_family, "Chosen Sans");
}
