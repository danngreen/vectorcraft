//! Export Faceplate…: a layer of artwork as a KiCad footprint, for a faceplate made as a PCB.
//!
//! The footprint (`.kicad_mod`) holds:
//! - the artwork layer as filled polygons on a KiCad layer (the silkscreen): what a high-resolution
//!   image of the layer converted with KiCad's Image Converter gives. The layer is drawn on white
//!   as an export draws it, a pixel is ink where it is darker than the threshold (over 50% grey,
//!   or any of its colour's inks over 50%: the lowest of its red, green and blue under half), and
//!   the ink's edge is followed through the anti-aliased pixels to a fraction of a pixel. So white
//!   art knocks out the art under it, and effects, gradients and images count as drawn;
//! - the shapes of the faceplate layer as closed outlines on Edge.Cuts, from their paths.
//!
//! Coordinates are millimetres from the centre of the first artboard, y down: those of the
//! positions CSV ([`super::positions`]), so the artwork, the board's edge and the controls line up.
//! Only art on that artboard is exported.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{PathEl, Rect};
use vectorcraft_render::encode::{RasterExportOptions, RasterFormat};

use super::fileio::{absolute_path, create_dir, write_file};
use super::{CommandSpec, Result, Session, bad, check_numbers, cmd, f64_or, has_doc, str_param};
use crate::EngineError;

/// Points per millimetre.
const MM: f64 = 72.0 / 25.4;
const ARTWORK_LAYER: &str = "artwork";
const FACEPLATE_LAYER: &str = "faceplate";
const KICAD_LAYER: &str = "F.SilkS";
const PPI: f64 = 2000.0;
/// The side of the tiles the layer is drawn in (pixels).
const TILE: u32 = 2048;
/// Most pixels drawn: a byte each is kept.
const MAX_PIXELS: f64 = 600.0e6;
/// How far a polygon's edge may lie from the ink's edge it follows (pixels).
const TOLERANCE_PX: f64 = 0.2;
/// Ink or a hole in it smaller than this is left out (square pixels).
const SPECK_PX: f64 = 2.0;
/// Curves of the faceplate outline are flattened to within this (mm).
const OUTLINE_TOLERANCE_MM: f64 = 0.002;
const OUTLINE_WIDTH_MM: f64 = 0.1;

type P = [f64; 2];

fn layer_named<'a>(doc: &'a Document, name: &str) -> Option<&'a Arc<Node>> {
    doc.layers.iter().find(|l| l.display_name().to_lowercase() == name.to_lowercase())
}

/// How light each pixel of `layer` is over `region`, drawn alone on white at `scale` pixels per
/// point: the lowest of its red, green and blue, a byte a pixel, row by row.
fn lightness(doc: &Document, layer: &Node, region: Rect, scale: f64, (w, h): (u32, u32)) -> Result<Vec<u8>> {
    let mut alone = layer.clone();
    alone.visible = true;
    let mut tmp = doc.clone();
    tmp.layers = vec![Arc::new(alone)];
    let opts = RasterExportOptions { ppi: scale * 72.0, background: Some([255; 3]), ..Default::default() }.render_options(RasterFormat::Png);
    let mut r = vectorcraft_render::Renderer::new();
    let mut out = vec![255u8; w as usize * h as usize];
    for ty in (0..h).step_by(TILE as usize) {
        for tx in (0..w).step_by(TILE as usize) {
            vectorcraft_doc::cancel::check()?;
            let (tw, th) = (TILE.min(w - tx), TILE.min(h - ty));
            let (x0, y0) = (region.x0 + f64::from(tx) / scale, region.y0 + f64::from(ty) / scale);
            let tile = Rect::new(x0, y0, x0 + f64::from(tw) / scale, y0 + f64::from(th) / scale);
            let img = r.render_region_with(&tmp, tile, scale, &opts);
            if (img.width, img.height) != (tw, th) {
                return Err(EngineError::Other("the artwork couldn't be drawn at this resolution".into()));
            }
            for (row, px) in img.pixels.chunks_exact(tw as usize * 4).enumerate() {
                let at = (ty as usize + row) * w as usize + tx as usize;
                for (o, [r, g, b, _]) in out[at..at + tw as usize].iter_mut().zip(px.as_chunks::<4>().0) {
                    *o = (*r).min(*g).min(*b);
                }
            }
        }
    }
    Ok(out)
}

/// The closed outlines of the area of `v` (`w` × `h` values at pixel centres, 255 beyond them)
/// that is under `level`, followed between the values (marching squares), in pixels. Each runs
/// with its area on the same side: an outline of ink has a positive [`area`], a hole a negative.
fn contours(v: &[u8], w: usize, h: usize, level: f64) -> Vec<Vec<P>> {
    let stride = (w + 2) as u64;
    let value = |x: usize, y: usize| -> f64 { if x == 0 || y == 0 || x > w || y > h { 255.0 } else { f64::from(v[(y - 1) * w + x - 1]) } };
    // Where an edge of the grid crosses the level: from → (to, where from is).
    let mut next: HashMap<u64, (u64, P)> = HashMap::new();
    let mut join = |a: (u64, P), b: (u64, P), corner: P, inside: bool| {
        let cross = (b.1[0] - a.1[0]) * (corner[1] - a.1[1]) - (b.1[1] - a.1[1]) * (corner[0] - a.1[0]);
        let (from, to) = if (cross > 0.0) == inside { (a, b) } else { (b, a) };
        next.insert(from.0, (to.0, from.1));
    };
    for cy in 0..=h {
        for cx in 0..=w {
            // The cell's corners: the values at (cx, cy), (cx+1, cy), (cx+1, cy+1), (cx, cy+1) of
            // the grid with its border, half a pixel up and left of the cell's own corner.
            let val = [value(cx, cy), value(cx + 1, cy), value(cx + 1, cy + 1), value(cx, cy + 1)];
            let ins = val.map(|a| a < level);
            if ins.iter().all(|i| *i == ins[0]) {
                continue;
            }
            let (x, y) = (cx as f64 - 0.5, cy as f64 - 0.5);
            let at = [[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]];
            let key = |gx: usize, gy: usize, upright: bool| (gy as u64 * stride + gx as u64) * 2 + u64::from(upright);
            // The crossing on the side from corner `i` to the next.
            let side = |i: usize| -> (u64, P) {
                let j = (i + 1) % 4;
                let t = (level - val[i]) / (val[j] - val[i]);
                let p = [at[i][0] + t * (at[j][0] - at[i][0]), at[i][1] + t * (at[j][1] - at[i][1])];
                let k = match i {
                    0 => key(cx, cy, false),
                    1 => key(cx + 1, cy, true),
                    2 => key(cx, cy + 1, false),
                    _ => key(cx, cy, true),
                };
                (k, p)
            };
            let crossed: Vec<usize> = (0..4).filter(|i| ins[*i] != ins[(*i + 1) % 4]).collect();
            if let [a, b] = crossed[..] {
                let i = ins.iter().position(|i| *i).unwrap_or(0);
                join(side(a), side(b), at[i], true);
            } else {
                // A saddle: its middle says which pair of corners is joined. The other two are
                // each cut off by a line of their own.
                let middle = val.iter().sum::<f64>() / 4.0 < level;
                let cut = if middle == ins[0] { [1, 3] } else { [0, 2] };
                for c in cut {
                    join(side((c + 3) % 4), side(c), at[c], ins[c]);
                }
            }
        }
    }
    let mut starts: Vec<u64> = next.keys().copied().collect();
    starts.sort_unstable();
    let mut out = vec![];
    for start in starts {
        let mut pts = vec![];
        let mut k = start;
        while let Some((to, p)) = next.remove(&k) {
            pts.push(p);
            k = to;
        }
        if pts.len() > 2 {
            out.push(pts);
        }
    }
    out
}

fn area(pts: &[P]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum::<f64>() / 2.0
}

fn distance_to_segment(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx * dx + dy * dy;
    let t = if len > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// The closed outline `pts` with the points left out that lie within `tol` of the line between
/// those kept (Douglas–Peucker, from its first point and the one farthest from it).
fn simplified(pts: &[P], tol: f64) -> Vec<P> {
    let n = pts.len();
    if n < 4 {
        return pts.to_vec();
    }
    let far = (1..n)
        .max_by(|a, b| (pts[*a][0] - pts[0][0]).hypot(pts[*a][1] - pts[0][1]).total_cmp(&(pts[*b][0] - pts[0][0]).hypot(pts[*b][1] - pts[0][1])));
    let far = far.unwrap_or(1);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let mut todo = vec![(0, far), (far, n)];
    while let Some((i, j)) = todo.pop() {
        let (a, b) = (pts[i], pts[j % n]);
        let mut worst = (tol, None);
        for (k, p) in pts.iter().enumerate().take(j).skip(i + 1) {
            let d = distance_to_segment(*p, a, b);
            if d > worst.0 {
                worst = (d, Some(k));
            }
        }
        if let (_, Some(k)) = worst {
            keep[k] = true;
            todo.push((i, k));
            todo.push((k, j));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn bounds(pts: &[P]) -> [f64; 4] {
    pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn holds(ring: &[P], p: P) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] <= p[1]) != (b[1] <= p[1]) && a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) > p[0] {
            inside = !inside;
        }
    }
    inside
}

/// `outer` with its `holes` cut in: one outline that reaches each hole along a seam of no width
/// from the hole's leftmost point straight left to the nearest edge (KiCad's polygons have no
/// holes; its Image Converter makes the same).
fn with_holes(outer: Vec<P>, mut holes: Vec<Vec<P>>) -> Vec<P> {
    let left = |h: &Vec<P>| h.iter().copied().enumerate().min_by(|a, b| a.1[0].total_cmp(&b.1[0])).unwrap_or((0, [0.0, 0.0]));
    holes.sort_by(|a, b| left(a).1[0].total_cmp(&left(b).1[0]));
    let mut ring = outer;
    for hole in holes {
        let (at, p) = left(&hole);
        let n = ring.len();
        let mut hit: Option<(usize, f64)> = None;
        for i in 0..n {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x <= p[0] && hit.is_none_or(|(_, best)| x > best) {
                    hit = Some((i, x));
                }
            }
        }
        let Some((i, x)) = hit else { continue };
        let seam = [x, p[1]];
        let mut joined = Vec::with_capacity(n + hole.len() + 3);
        joined.extend_from_slice(&ring[..=i]);
        joined.push(seam);
        joined.extend_from_slice(&hole[at..]);
        joined.extend_from_slice(&hole[..=at]);
        joined.push(seam);
        joined.extend_from_slice(&ring[i + 1..]);
        ring = joined;
    }
    ring
}

/// The polygons of the ink whose `loops` these are: each outline of ink with the holes in it cut in.
fn polygons(loops: Vec<Vec<P>>) -> Vec<Vec<P>> {
    let (mut outers, mut holes) = (vec![], vec![]);
    for l in loops {
        let a = area(&l);
        if a.abs() < SPECK_PX || l.len() < 3 {
            continue;
        }
        if a > 0.0 { outers.push((l, a, vec![])) } else { holes.push((l, -a)) }
    }
    let boxes: Vec<[f64; 4]> = outers.iter().map(|o| bounds(&o.0)).collect();
    for (hole, a) in holes {
        let p = hole[0];
        // The smallest outline of ink around it.
        let parent = (0..outers.len())
            .filter(|i| outers[*i].1 > a && boxes[*i][0] <= p[0] && p[0] <= boxes[*i][2] && boxes[*i][1] <= p[1] && p[1] <= boxes[*i][3])
            .filter(|i| holds(&outers[*i].0, p))
            .min_by(|a, b| outers[*a].1.total_cmp(&outers[*b].1));
        if let Some(i) = parent {
            outers[i].2.push(hole);
        }
    }
    outers.into_iter().map(|(outer, _, holes)| if holes.is_empty() { outer } else { with_holes(outer, holes) }).collect()
}

/// The closed outlines of the shapes on `layer` that lie at least partly on `board`, flattened, in
/// points; and how many open paths were left out.
fn outlines(doc: &Document, layer: &Node, board: Rect) -> (Vec<Vec<P>>, usize) {
    let (mut out, mut open) = (vec![], 0);
    for child in layer.children().into_iter().flatten() {
        child.walk(&mut |n: &Node| {
            if n.children().is_some() || !n.visible {
                return;
            }
            let Some((path, _)) = vectorcraft_render::drawn_outline(doc, n) else { return };
            let mut sub: Vec<P> = vec![];
            let mut end = |sub: &mut Vec<P>, closed: bool| {
                if sub.len() > 1 && sub.first() == sub.last() {
                    sub.pop();
                }
                let b = bounds(sub);
                let on_board = b[0] < board.x1 && b[2] > board.x0 && b[1] < board.y1 && b[3] > board.y0;
                if closed && sub.len() > 2 {
                    if on_board {
                        out.push(std::mem::take(sub));
                    }
                } else if sub.len() > 1 && on_board {
                    open += 1;
                }
                sub.clear();
            };
            vectorcraft_geom::flatten(path.elements().iter().copied(), OUTLINE_TOLERANCE_MM * MM, |el| match el {
                PathEl::MoveTo(p) => {
                    end(&mut sub, false);
                    sub.push([p.x, p.y]);
                }
                PathEl::LineTo(p) => sub.push([p.x, p.y]),
                PathEl::ClosePath => end(&mut sub, true),
                _ => {}
            });
            end(&mut sub, false);
        });
    }
    (out, open)
}

fn mm(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// `v` mm to the file's four decimals.
fn tenth_um(v: f64) -> f64 {
    (v * 1e4).round() / 1e4 + 0.0
}

fn poly(out: &mut String, pts: &[P], style: &str, layer: &str) {
    out.push_str("  (fp_poly\n    (pts");
    for (i, p) in pts.iter().enumerate() {
        let _ = write!(out, "{}(xy {} {})", if i % 4 == 0 { "\n      " } else { " " }, mm(p[0]), mm(p[1]));
    }
    let _ = writeln!(out, "\n    )\n    {style} (layer \"{layer}\"))");
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The footprint a document saved at `file` exports to when no path is given: its own name in
/// `artwork.pretty` (a KiCad footprint library) next to it.
fn default_path(file: Option<&str>) -> Option<String> {
    let file = std::path::Path::new(file?);
    let stem = file.file_stem()?.to_string_lossy().into_owned();
    Some(file.with_file_name("artwork.pretty").join(format!("{stem}.kicad_mod")).to_string_lossy().into_owned())
}

/// What Export Faceplate uses where its params say nothing: {path ("" for a document not saved
/// yet), layer, faceplateLayer, ppi}.
pub fn defaults(s: &Session) -> Value {
    let path = s.doc().ok().and_then(|st| default_path(st.path.as_deref())).unwrap_or_default();
    json!({ "path": path, "layer": ARTWORK_LAYER, "faceplateLayer": FACEPLATE_LAYER, "ppi": PPI })
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "faceplate.export";
    check_numbers(p, &["ppi", "threshold"], C)?;
    let ppi = f64_or(p, "ppi", PPI);
    if !(72.0..=4000.0).contains(&ppi) {
        return Err(bad(C, "ppi must be between 72 and 4000"));
    }
    let threshold = f64_or(p, "threshold", 50.0);
    if !(threshold > 0.0 && threshold < 100.0) {
        return Err(bad(C, "threshold must be between 0 and 100 (percent)"));
    }
    let (art_name, plate_name) = (str_param(p, "layer").unwrap_or(ARTWORK_LAYER), str_param(p, "faceplateLayer").unwrap_or(FACEPLATE_LAYER));
    let kicad_layer = str_param(p, "kicadLayer").unwrap_or(KICAD_LAYER);
    let st = s.doc()?;
    let doc = st.doc.clone();
    let path = match str_param(p, "path") {
        Some(path) => absolute_path(path),
        None => default_path(st.path.as_deref()).ok_or_else(|| bad(C, "the document isn't saved yet: pass path, the .kicad_mod file to write"))?,
    };
    let name = str_param(p, "name")
        .map(String::from)
        .or_else(|| std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "faceplate".into());
    let board = doc.artboards.first().map(|a| a.rect).ok_or_else(|| bad(C, "the document has no artboard"))?;
    let origin = board.center();
    let art = layer_named(&doc, art_name).ok_or_else(|| bad(C, format!("the document has no layer named {art_name}")))?;
    let mut warnings: Vec<String> = vec![];

    // The artwork: what it draws on the artboard, in whole pixels.
    let scale = ppi / 72.0;
    let drawn = vectorcraft_render::drawn_bounds(&doc, art).map(|b| b.intersect(board)).filter(|b| b.width() > 0.0 && b.height() > 0.0);
    let mut shapes: Vec<Vec<P>> = vec![];
    if let Some(b) = drawn {
        let (w, h) = ((b.width() * scale).ceil().max(1.0), (b.height() * scale).ceil().max(1.0));
        if w * h > MAX_PIXELS {
            return Err(bad(C, format!("the artwork is {w} × {h} pixels at {ppi} ppi, too many: lower ppi")));
        }
        let size = (w as u32, h as u32);
        let region = Rect::new(b.x0, b.y0, b.x0 + w / scale, b.y0 + h / scale);
        let light = lightness(&doc, art, region, scale, size)?;
        let level = 255.0 * (1.0 - threshold / 100.0);
        let loops = contours(&light, size.0 as usize, size.1 as usize, level).iter().map(|l| simplified(l, TOLERANCE_PX)).collect();
        let to_mm = |p: &P| [(region.x0 + p[0] / scale - origin.x) / MM, (region.y0 + p[1] / scale - origin.y) / MM];
        shapes = polygons(loops).iter().map(|ring| ring.iter().map(to_mm).collect()).collect();
    }
    if shapes.is_empty() {
        warnings.push(format!("the {art_name} layer draws nothing dark on the artboard, so the footprint has no artwork"));
    }

    // The faceplate's outline.
    let mut edges: Vec<Vec<P>> = vec![];
    match layer_named(&doc, plate_name) {
        Some(plate) => {
            let (found, open) = outlines(&doc, plate, board);
            edges = found.iter().map(|ring| ring.iter().map(|p| [(p[0] - origin.x) / MM, (p[1] - origin.y) / MM]).collect()).collect();
            if open > 0 {
                warnings.push(format!("{plate_name} layer: {open} open path{} can't be an outline, so left out", if open == 1 { "" } else { "s" }));
            }
            if edges.is_empty() {
                warnings.push(format!("{plate_name} layer: no closed shape on the artboard, so the footprint has no board outline"));
            }
        }
        None => warnings.push(format!("no layer named {plate_name}, so the footprint has no board outline")),
    }
    let outline = edges.iter().map(|e| bounds(e)).max_by(|a, b| ((a[2] - a[0]) * (a[3] - a[1])).total_cmp(&((b[2] - b[0]) * (b[3] - b[1]))));

    let mut out = String::new();
    let _ = writeln!(out, "(footprint {} (version 20221018) (generator \"vectorcraft\")", quoted(&name));
    out.push_str("  (layer \"F.Cu\")\n  (attr board_only exclude_from_pos_files exclude_from_bom)\n");
    for (kind, text) in [("reference", "G***"), ("value", name.as_str())] {
        let _ = writeln!(
            out,
            "  (fp_text {kind} {} (at 0 0) (layer \"F.Fab\") hide\n    (effects (font (size 1.5 1.5) (thickness 0.3)))\n  )",
            quoted(text)
        );
    }
    for e in &edges {
        poly(&mut out, e, &format!("(stroke (width {OUTLINE_WIDTH_MM}) (type solid)) (fill none)"), "Edge.Cuts");
    }
    for shape in &shapes {
        poly(&mut out, shape, "(stroke (width 0) (type solid)) (fill solid)", kicad_layer);
    }
    out.push_str(")\n");
    if let Some(dir) = std::path::Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
        create_dir(&dir.to_string_lossy())?;
    }
    write_file(&path, out.as_bytes())?;
    Ok(json!({
        "path": path, "name": name, "layer": art_name, "kicadLayer": kicad_layer, "ppi": ppi,
        "polygons": shapes.len(), "points": shapes.iter().map(Vec::len).sum::<usize>(),
        "outlines": edges.len(),
        "outline": outline.map(|b| json!({ "x": tenth_um((b[0] + b[2]) / 2.0), "y": tenth_um((b[1] + b[3]) / 2.0), "w": tenth_um(b[2] - b[0]), "h": tenth_um(b[3] - b[1]) })),
        "warnings": warnings,
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "faceplate.export",
        "Export Faceplate…",
        ["File", "Export"],
        None,
        "{path? (the .kicad_mod to write; default artwork.pretty/<document name>.kicad_mod next to the document), layer? (the artwork's, default artwork), faceplateLayer? (the panel's outline, default faceplate), kicadLayer? (default F.SilkS), ppi? (the resolution the artwork is read at, default 2000), threshold? (percent, default 50: darker is ink), name? (the footprint's, default the file's)} write a layer's artwork as a KiCad footprint for a faceplate PCB: filled polygons of what the layer draws darker than the threshold (as a high-resolution image of it converted with KiCad's Image Converter: white art knocks out what is under it), and the faceplate layer's closed shapes as Edge.Cuts outlines; mm from the first artboard's centre, y down, as positions.export → {path, name, layer, kicadLayer, ppi, polygons, points, outlines, outline: the largest's {x, y, w, h} mm|null, warnings}",
        has_doc,
        export
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field of `w` × `h` values, 0 where `ink` says and 255 elsewhere.
    fn field(w: usize, h: usize, ink: impl Fn(usize, usize) -> bool) -> Vec<u8> {
        (0..w * h).map(|i| if ink(i % w, i / w) { 0 } else { 255 }).collect()
    }

    #[test]
    fn a_square_of_ink_is_one_outline_around_its_pixels() {
        let v = field(8, 8, |x, y| (2..6).contains(&x) && (2..6).contains(&y));
        let loops: Vec<Vec<P>> = contours(&v, 8, 8, 127.5).iter().map(|l| simplified(l, TOLERANCE_PX)).collect();
        assert_eq!(loops.len(), 1);
        // The edge lies half-way between an ink pixel's centre and its white neighbour's: on the
        // pixels' own edge, the corners cut by half a pixel.
        assert!((area(&loops[0]) - 15.5).abs() < 0.01, "{}", area(&loops[0]));
        let b = bounds(&loops[0]);
        assert_eq!(b, [2.0, 2.0, 6.0, 6.0]);
    }

    #[test]
    fn a_hole_is_cut_into_its_outline_and_ink_inside_it_stands_alone() {
        // A ring of ink with a dot of ink in its hole.
        let v = field(20, 20, |x, y| {
            let ring = (2..18).contains(&x) && (2..18).contains(&y) && !((5..15).contains(&x) && (5..15).contains(&y));
            ring || ((8..12).contains(&x) && (8..12).contains(&y))
        });
        let loops: Vec<Vec<P>> = contours(&v, 20, 20, 127.5).iter().map(|l| simplified(l, TOLERANCE_PX)).collect();
        assert_eq!(loops.iter().filter(|l| area(l) > 0.0).count(), 2);
        assert_eq!(loops.iter().filter(|l| area(l) < 0.0).count(), 1);
        let ink: f64 = loops.iter().map(|l| area(l)).sum();
        let polys = polygons(loops);
        assert_eq!(polys.len(), 2);
        // The seam has no width: the polygons' areas are the ink's.
        assert!((polys.iter().map(|p| area(p)).sum::<f64>() - ink).abs() < 1e-9);
        // The ring's polygon goes out to the hole and back along the seam.
        let ring = polys.iter().max_by(|a, b| a.len().cmp(&b.len())).unwrap();
        assert!(ring.windows(2).any(|w| w[0][1] == w[1][1] && (w[0][0] - w[1][0]).abs() > 2.0));
    }

    #[test]
    fn specks_are_left_out() {
        let v = field(8, 8, |x, y| x == 3 && y == 3);
        let loops = contours(&v, 8, 8, 127.5);
        assert_eq!(loops.len(), 1);
        assert!(polygons(loops).is_empty());
    }
}
