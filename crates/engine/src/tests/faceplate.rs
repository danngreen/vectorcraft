//! Export Faceplate: a layer's artwork and the faceplate's outline as a KiCad footprint.

use serde_json::{Value, json};

use crate::*;

const MM: f64 = 72.0 / 25.4;

/// A 100 × 60 mm document with layers `artwork` and `faceplate`.
fn panel() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100.0 * MM, "height": 60.0 * MM})).unwrap();
    for name in ["faceplate", "artwork"] {
        s.execute("layer.new", &json!({"name": name})).unwrap();
    }
    s
}

fn on(s: &mut Session, layer: &str) {
    let id = s.doc().unwrap().doc.layers.iter().find(|l| l.display_name() == layer).unwrap().id;
    s.execute("layer.setCurrent", &json!({"id": id.0})).unwrap();
}

/// A rectangle centred on (cx, cy) mm from the artboard's corner, filled with `fill`, no stroke.
fn rect(s: &mut Session, cx: f64, cy: f64, w: f64, h: f64, fill: &str) {
    s.execute("shape.rectangle", &json!({"x": (cx - w / 2.0) * MM, "y": (cy - h / 2.0) * MM, "width": w * MM, "height": h * MM})).unwrap();
    s.execute("paint.setFill", &json!({"color": fill})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
}

/// The polygons of `layer` in the footprint `text`: their points, mm.
fn polys(text: &str, layer: &str) -> Vec<Vec<[f64; 2]>> {
    text.split("(fp_poly")
        .skip(1)
        .filter(|b| b.contains(&format!("(layer \"{layer}\")")))
        .map(|b| {
            b.split("(xy ")
                .skip(1)
                .map(|p| {
                    let mut n = p.split([' ', ')']).map(|v| v.parse::<f64>().unwrap());
                    [n.next().unwrap(), n.next().unwrap()]
                })
                .collect()
        })
        .collect()
}

fn area(p: &[[f64; 2]]) -> f64 {
    (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f64>().abs() / 2.0
}

fn bounds(p: &[[f64; 2]]) -> [f64; 4] {
    p.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn export(s: &mut Session, name: &str, params: Value) -> (Value, String) {
    let dir = std::env::temp_dir().join(format!("vc-faceplate-{name}-{}", std::process::id()));
    let path = dir.join("art.pretty").join("panel.kicad_mod").to_string_lossy().into_owned();
    let mut p = params;
    p["path"] = json!(path);
    let out = s.execute("faceplate.export", &p).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out, text)
}

#[test]
fn dark_art_is_silkscreen_white_art_knocks_it_out_and_the_faceplate_shape_is_the_edge() {
    let mut s = panel();
    on(&mut s, "faceplate");
    rect(&mut s, 50.0, 30.0, 90.0, 50.0, "#cccccc");
    on(&mut s, "artwork");
    // Black with a white window in it, then blocks of red, dark grey and yellow (ink: one of their
    // colour's inks is over half) and of light grey (not ink).
    rect(&mut s, 30.0, 30.0, 20.0, 10.0, "#000000");
    rect(&mut s, 30.0, 30.0, 10.0, 4.0, "#ffffff");
    rect(&mut s, 60.0, 20.0, 6.0, 6.0, "#ff0000");
    rect(&mut s, 70.0, 20.0, 6.0, 6.0, "#606060");
    rect(&mut s, 80.0, 20.0, 6.0, 6.0, "#a0a0a0");
    rect(&mut s, 60.0, 40.0, 6.0, 6.0, "#ffff00");
    let (out, text) = export(&mut s, "knockout", json!({}));
    assert_eq!(out["warnings"], json!([]), "{out}");
    assert!(text.starts_with("(footprint \"panel\" (version 20221018) (generator \"vectorcraft\")"), "{}", &text[..80]);

    // The edge: the faceplate's rectangle, from the artboard's centre.
    let edge = polys(&text, "Edge.Cuts");
    assert_eq!(edge.len(), 1);
    assert_eq!(bounds(&edge[0]).map(|v| (v * 1000.0).round() / 1000.0), [-45.0, -25.0, 45.0, 25.0]);
    assert_eq!(out["outline"], json!({"x": 0.0, "y": 0.0, "w": 90.0, "h": 50.0}));

    // The ink: the black block less its window (one polygon, the window cut in), and the red, the
    // dark grey and the yellow blocks.
    let mut silk = polys(&text, "F.SilkS");
    silk.sort_by(|a, b| area(b).total_cmp(&area(a)));
    assert_eq!(silk.len(), 4, "{out}");
    // (Edges are right to a few hundredths of a pixel: about a micron at 1000 ppi.)
    assert!((area(&silk[0]) - (200.0 - 40.0)).abs() < 0.2, "{}", area(&silk[0]));
    let b = bounds(&silk[0]);
    for (got, want) in b.iter().zip([-30.0, -5.0, -10.0, 5.0]) {
        assert!((got - want).abs() < 0.01, "{b:?}");
    }
    for block in &silk[1..] {
        // (A grey's edge is where its anti-aliased pixels pass the threshold: within half a pixel.)
        assert!((area(block) - 36.0).abs() < 0.3, "{}", area(block));
    }
    let centres: Vec<(i32, i32)> =
        silk[1..].iter().map(|p| bounds(p)).map(|b| (((b[0] + b[2]) / 2.0).round() as i32, ((b[1] + b[3]) / 2.0).round() as i32)).collect();
    for want in [(10, -10), (20, -10), (10, 10)] {
        assert!(centres.contains(&want), "{centres:?}");
    }
}

#[test]
fn the_layers_can_be_named_and_missing_ones_are_said() {
    let mut s = panel();
    on(&mut s, "artwork");
    rect(&mut s, 50.0, 30.0, 10.0, 10.0, "#000000");
    // No such artwork layer: an error.
    let e = s.execute("faceplate.export", &json!({"path": "/nonexistent/x.kicad_mod", "layer": "silk"})).unwrap_err();
    assert!(e.to_string().contains("no layer named silk"), "{e}");
    // A faceplate layer without a shape: the footprint has the artwork and says what it lacks.
    let (out, text) = export(&mut s, "missing", json!({}));
    assert_eq!(out["polygons"], json!(1));
    assert_eq!(out["outline"], Value::Null);
    assert!(out["warnings"][0].as_str().unwrap().contains("no closed shape"), "{out}");
    assert!(polys(&text, "Edge.Cuts").is_empty());
    // Another layer as the artwork, onto another KiCad layer.
    let (out, text) = export(&mut s, "named", json!({"layer": "Artwork", "faceplateLayer": "artwork", "kicadLayer": "F.Mask"}));
    assert_eq!((out["polygons"].as_u64(), out["outlines"].as_u64()), (Some(1), Some(1)), "{out}");
    assert_eq!(polys(&text, "F.Mask").len(), 1);
}
