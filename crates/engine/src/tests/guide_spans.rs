//! Guide spans: parallel ruler guides selected count in the box `object.setBounds` sets, alone or
//! with art.

use serde_json::{Value, json};

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn positions(s: &Session) -> Vec<f64> {
    s.doc().unwrap().doc.ruler_guides().iter().map(|g| g.pos).collect()
}

fn art_box(s: &Session) -> vectorcraft_geom::Rect {
    let st = s.doc().unwrap();
    s.transform_box(&st.selection.objects).unwrap().rect
}

#[test]
fn parallel_guides_alone_move_and_scale_about_the_reference_point() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 200}));
    for pos in [43, 50, 100] {
        run(&mut s, "guide.add", json!({"vertical": true, "pos": pos}));
    }
    run(&mut s, "guide.select", json!({"indexes": [0, 1, 2]}));
    assert_eq!(s.guide_span(None), Some(GuideSpan { vertical: true, lo: 43.0, hi: 100.0 }));
    // X places the reference point: the left guide, the middle of the span or the right guide.
    run(&mut s, "object.setBounds", json!({"x": 10, "reference": 0}));
    assert_eq!(positions(&s), [10.0, 17.0, 67.0]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(positions(&s), [43.0, 50.0, 100.0], "one undo step");
    run(&mut s, "object.setBounds", json!({"x": 81.5, "reference": 7}));
    assert_eq!(positions(&s), [53.0, 60.0, 110.0]);
    run(&mut s, "edit.undo", json!({}));
    // W scales the spacing about the reference point.
    run(&mut s, "object.setBounds", json!({"width": 114, "reference": 3}));
    assert_eq!(positions(&s), [43.0, 57.0, 157.0]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "object.setBounds", json!({"width": 28.5, "reference": 8}));
    assert_eq!(positions(&s), [71.5, 75.0, 100.0]);
    run(&mut s, "edit.undo", json!({}));
    // Alt+Enter: scaled copies, selected; the guides stay.
    run(&mut s, "object.setBounds", json!({"width": 114, "reference": 0, "copy": true}));
    assert_eq!(positions(&s), [43.0, 50.0, 100.0, 43.0, 57.0, 157.0]);
    assert_eq!(super::selected_guides(&s), [3, 4, 5]);
    run(&mut s, "edit.undo", json!({}));
    // Vertical guides alone have no Y or H.
    run(&mut s, "guide.select", json!({"indexes": [0, 1, 2]}));
    for p in [json!({"y": 10}), json!({"height": 10})] {
        let e = s.execute("object.setBounds", &p).unwrap_err().to_string();
        assert!(e.contains("vertical"), "{e}");
    }
    assert_eq!(positions(&s), [43.0, 50.0, 100.0]);
    // One guide has no width to scale, and moves.
    run(&mut s, "guide.select", json!({"indexes": [1]}));
    run(&mut s, "object.setBounds", json!({"width": 30, "x": 55}));
    assert_eq!(positions(&s), [43.0, 55.0, 100.0]);
}

#[test]
fn horizontal_guides_with_art_scale_with_its_box() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 0, "width": 30, "height": 40}));
    run(&mut s, "guide.add", json!({"vertical": false, "pos": 60}));
    run(&mut s, "guide.add", json!({"vertical": false, "pos": 100}));
    run(&mut s, "guide.select", json!({"indexes": [0, 1], "toggle": true}));
    let art = s.transform_box(&s.doc().unwrap().selection.objects);
    let span = s.guide_span(art.as_ref()).unwrap();
    assert_eq!(span.bounds(art.map(|b| b.rect)), vectorcraft_geom::Rect::new(10.0, 0.0, 40.0, 100.0));
    // H scales the art and the guides down from the top; the art keeps its width.
    run(&mut s, "object.setBounds", json!({"height": 200, "reference": 0}));
    assert_eq!(positions(&s), [120.0, 200.0]);
    assert_eq!(art_box(&s), vectorcraft_geom::Rect::new(10.0, 0.0, 40.0, 80.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((positions(&s), art_box(&s)), (vec![60.0, 100.0], vectorcraft_geom::Rect::new(10.0, 0.0, 40.0, 40.0)), "one undo step");
    // Proportional, from the bottom right: the art's width follows the whole box's height.
    run(&mut s, "object.setBounds", json!({"height": 50, "reference": 8, "proportional": true}));
    assert_eq!(positions(&s), [80.0, 100.0]);
    assert_eq!(art_box(&s), vectorcraft_geom::Rect::new(25.0, 50.0, 40.0, 70.0));
    run(&mut s, "edit.undo", json!({}));
    // X and W act on the art alone, as without guides.
    run(&mut s, "object.setBounds", json!({"x": 100, "width": 60, "reference": 0}));
    assert_eq!(positions(&s), [60.0, 100.0]);
    assert_eq!(art_box(&s), vectorcraft_geom::Rect::new(100.0, 0.0, 160.0, 40.0));
}

#[test]
fn crossing_guides_or_turned_art_keep_the_art_box() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    run(&mut s, "guide.add", json!({"vertical": true, "pos": 60}));
    run(&mut s, "guide.add", json!({"vertical": false, "pos": 100}));
    run(&mut s, "guide.select", json!({"indexes": [0, 1]}));
    assert_eq!(s.guide_span(None), None);
    // The art's own box, as before.
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 0, "width": 30, "height": 40}));
    run(&mut s, "guide.select", json!({"indexes": [0, 1], "toggle": true}));
    run(&mut s, "object.setBounds", json!({"width": 60, "reference": 0}));
    assert_eq!(positions(&s), [60.0, 100.0]);
    assert_eq!(art_box(&s), vectorcraft_geom::Rect::new(10.0, 0.0, 70.0, 40.0));
    // Turned art: parallel guides don't count either.
    run(&mut s, "guide.select", json!({"indexes": [0], "toggle": true}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let art = s.transform_box(&s.doc().unwrap().selection.objects);
    assert!(art.is_some_and(|b| b.angle != 0.0));
    assert_eq!(s.guide_span(art.as_ref()), None);
}
