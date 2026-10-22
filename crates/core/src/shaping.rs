//! Shaping: weld, trim, intersect, simplify, front-minus-back, back-minus-
//! front and boundary. Clearly separated or nested simple closed paths
//! retain their original curves. Simple crossing paths can retain their
//! segments with bounded parametric clipping, while complex/ambiguous
//! cases still use flattened polygons (0.02 mm tolerance).

use crate::geometry::{BezPath, PathEl, Point};
use i_overlay::core::fill_rule::FillRule as IFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use kurbo::{ParamCurve as _, ParamCurveArclen as _};

const TOL: f64 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Weld,
    Intersect,
    /// Subject minus clip.
    Trim,
    /// Symmetric difference.
    Exclude,
}

/// Flatten a path into closed contours (one per subpath).
pub fn contours(path: &BezPath) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), TOL, &mut |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() >= 3 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur.push([p.x, p.y]);
        }
        PathEl::LineTo(p) => cur.push([p.x, p.y]),
        PathEl::ClosePath => {}
        _ => {}
    });
    if cur.len() >= 3 {
        out.push(cur);
    }
    out
}

/// Build a path from shapes (outer contour + holes each).
fn from_shapes(shapes: Vec<Vec<Vec<[f64; 2]>>>) -> BezPath {
    let mut path = BezPath::new();
    for shape in shapes {
        for contour in shape {
            let mut first = true;
            for p in contour {
                let pt = Point::new(p[0], p[1]);
                if first {
                    path.move_to(pt);
                    first = false;
                } else {
                    path.line_to(pt);
                }
            }
            path.close_path();
        }
    }
    path
}

/// Join separate paths without replacing their original control points.
fn join_exact_paths(paths: &[BezPath]) -> BezPath {
    let mut out = BezPath::new();
    for path in paths {
        for el in path.elements() {
            out.push(*el);
        }
    }
    out
}

/// Euclidean distance from a point to a chord, squared.
fn point_chord_distance_sq(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len_sq = dx * dx + dy * dy;
    let t = (if len_sq > 1e-24 {
        ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len_sq
    } else {
        0.0
    })
    .clamp(0.0, 1.0);
    let x = a[0] + dx * t - p[0];
    let y = a[1] + dy * t - p[1];
    x * x + y * y
}

fn chord_side(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// True when two polygonal chords cross or are too close to trust the
/// flattened geometry. A bounding-box test alone rejects safe diagonal
/// segments, so check the actual chord distance when the boxes overlap.
/// Flattening uses TOL per input path: require 2*TOL clearance.
fn chord_bounds_near(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let gap = 2.0 * TOL + 1e-7;
    let (ax0, ax1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (ay0, ay1) = (a[1].min(b[1]), a[1].max(b[1]));
    let (bx0, bx1) = (c[0].min(d[0]), c[0].max(d[0]));
    let (by0, by1) = (c[1].min(d[1]), c[1].max(d[1]));
    if ax0 > bx1 + gap
        || bx0 > ax1 + gap
        || ay0 > by1 + gap
        || by0 > ay1 + gap
    {
        return false;
    }

    let ab_c = chord_side(a, b, c);
    let ab_d = chord_side(a, b, d);
    let cd_a = chord_side(c, d, a);
    let cd_b = chord_side(c, d, b);
    if (ab_c > 0.0 && ab_d < 0.0 || ab_c < 0.0 && ab_d > 0.0)
        && (cd_a > 0.0 && cd_b < 0.0 || cd_a < 0.0 && cd_b > 0.0)
    {
        return true;
    }

    let gap_sq = gap * gap;
    point_chord_distance_sq(a, c, d) <= gap_sq
        || point_chord_distance_sq(b, c, d) <= gap_sq
        || point_chord_distance_sq(c, a, b) <= gap_sq
        || point_chord_distance_sq(d, a, b) <= gap_sq
}

fn signed_polygon_area(contour: &[[f64; 2]]) -> f64 {
    contour
        .iter()
        .zip(contour.iter().cycle().skip(1))
        .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
        .sum::<f64>()
        / 2.0
}

/// Permit only one finite, simple, closed contour. Multi-contour and
/// self-intersecting paths keep using the general polygon overlay algorithm.
fn simple_closed_contour(path: &BezPath) -> Option<Vec<[f64; 2]>> {
    let els = path.elements();
    if !matches!(els.first(), Some(PathEl::MoveTo(_)))
        || !matches!(els.last(), Some(PathEl::ClosePath))
        || els.iter().filter(|el| matches!(el, PathEl::MoveTo(_))).count() != 1
        || els.iter().filter(|el| matches!(el, PathEl::ClosePath)).count() != 1
    {
        return None;
    }
    // Bound the fast path to a realistic mm coordinate range. This keeps
    // squared-distance and signed-area checks numerically meaningful.
    let finite = |p: Point| {
        p.x.is_finite()
            && p.y.is_finite()
            && p.x.abs() <= 1_000_000.0
            && p.y.abs() <= 1_000_000.0
    };
    if !els.iter().all(|el| match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => finite(p),
        PathEl::QuadTo(a, b) => finite(a) && finite(b),
        PathEl::CurveTo(a, b, c) => finite(a) && finite(b) && finite(c),
        PathEl::ClosePath => true,
    }) {
        return None;
    }
    let mut all = contours(path);
    if all.len() != 1 {
        return None;
    }
    let mut contour = all.pop()?;
    if let (Some(first), Some(last)) = (contour.first(), contour.last()) {
        if (first[0] - last[0]).abs() <= 1e-9
            && (first[1] - last[1]).abs() <= 1e-9
        {
            contour.pop();
        }
    }
    let n = contour.len();
    // Keep this conservative fast path inexpensive for high-node artwork.
    if !(3..=512).contains(&n) || signed_polygon_area(&contour).abs() <= 1e-8 {
        return None;
    }
    for i in 0..n {
        for j in (i + 1)..n {
            // Adjacent chords share endpoints, including the closing seam.
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if chord_bounds_near(
                contour[i],
                contour[(i + 1) % n],
                contour[j],
                contour[(j + 1) % n],
            ) {
                return None;
            }
        }
    }
    Some(contour)
}

fn polygon_contains(contour: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    for (a, b) in contour.iter().zip(contour.iter().cycle().skip(1)) {
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < a[0] + (point[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

fn reverse_closed_contour(path: &BezPath) -> Option<BezPath> {
    let subpaths = subpath_segments(path);
    let (segs, closed) = subpaths.first()?;
    if subpaths.len() != 1 || !*closed {
        return None;
    }
    let mut out = BezPath::new();
    out.move_to(segs.last()?.end());
    for seg in segs.iter().rev() {
        match seg {
            kurbo::PathSeg::Line(line) => out.line_to(line.p0),
            kurbo::PathSeg::Quad(quad) => out.quad_to(quad.p1, quad.p0),
            kurbo::PathSeg::Cubic(cubic) => {
                out.curve_to(cubic.p2, cubic.p1, cubic.p0);
            }
        }
    }
    out.close_path();
    Some(out)
}

/// A nested difference is a compound path. Reverse the inner boundary when
/// needed so both even-odd and non-zero fill rules recognise the hole.
fn exact_hole(
    outer: &BezPath,
    inner: &BezPath,
    outer_poly: &[[f64; 2]],
    inner_poly: &[[f64; 2]],
) -> Option<BezPath> {
    let inner = if signed_polygon_area(outer_poly).signum()
        == signed_polygon_area(inner_poly).signum()
    {
        reverse_closed_contour(inner)?
    } else {
        inner.clone()
    };
    Some(join_exact_paths(&[outer.clone(), inner]))
}

/// Retain exact cubic control points when two uncomplicated closed paths
/// have a clear gap, or one lies strictly inside the other. This does not
/// replace the polygon algorithm for crossing, touching or complex paths.
fn exact_non_crossing_parts(
    subject: &BezPath,
    clip: &BezPath,
    op: Op,
) -> Option<Vec<BezPath>> {
    use crate::geometry::Shape as _;
    // Identical closed contours need no curve subdivision or polygon
    // reconstruction. Difference and XOR cancel exactly.
    if subject.elements() == clip.elements() && simple_closed_contour(subject).is_some() {
        return Some(match op {
            Op::Weld | Op::Intersect => vec![subject.clone()],
            Op::Trim | Op::Exclude => Vec::new(),
        });
    }
    let margin = 2.0 * TOL + 1e-7;
    let sb = subject.bounding_box();
    let cb = clip.bounding_box();
    let separate = sb.x1 + margin < cb.x0
        || cb.x1 + margin < sb.x0
        || sb.y1 + margin < cb.y0
        || cb.y1 + margin < sb.y0;
    // Overlapping bounds do not imply touching curves: two diagonally
    // separated shapes can share an axis-aligned bounding rectangle.
    // Validate actual chord clearance before classifying their topology.
    let a = simple_closed_contour(subject)?;
    let b = simple_closed_contour(clip)?;
    if !separate {
        for i in 0..a.len() {
            for j in 0..b.len() {
                if chord_bounds_near(
                    a[i],
                    a[(i + 1) % a.len()],
                    b[j],
                    b[(j + 1) % b.len()],
                ) {
                    return None;
                }
            }
        }
    }
    let subject_inside = !separate && polygon_contains(&b, a[0]);
    let clip_inside = !separate && polygon_contains(&a, b[0]);
    if subject_inside && clip_inside {
        return None;
    }
    match (subject_inside, clip_inside, op) {
        (false, false, Op::Weld | Op::Exclude) => {
            Some(vec![subject.clone(), clip.clone()])
        }
        (false, false, Op::Trim) => Some(vec![subject.clone()]),
        (false, false, Op::Intersect) => Some(Vec::new()),
        (true, false, Op::Weld) => Some(vec![clip.clone()]),
        (true, false, Op::Intersect) => Some(vec![subject.clone()]),
        (true, false, Op::Trim) => Some(Vec::new()),
        (true, false, Op::Exclude) => {
            Some(vec![exact_hole(clip, subject, &b, &a)?])
        }
        (false, true, Op::Weld) => Some(vec![subject.clone()]),
        (false, true, Op::Intersect) => Some(vec![clip.clone()]),
        (false, true, Op::Trim | Op::Exclude) => {
            Some(vec![exact_hole(subject, clip, &a, &b)?])
        }
        _ => None,
    }
}

/// An intersection on a segment, shared between both input boundaries.
#[derive(Clone, Copy)]
struct CurveCut {
    segment: usize,
    t: f64,
    vertex: usize,
}

struct CurveEdge {
    from: usize,
    to: usize,
    pieces: Vec<kurbo::PathSeg>,
    inside_other: bool,
}

/// Walk the original path between successive intersections. The path's
/// cubic and quadratic pieces are split at parameter values, not flattened.
fn crossing_arcs(
    segments: &[kurbo::PathSeg],
    cuts: &[CurveCut],
    other: &BezPath,
) -> Option<Vec<CurveEdge>> {
    let mut pieces: Vec<(Option<usize>, Option<usize>, kurbo::PathSeg)> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let mut hits: Vec<_> = cuts
            .iter()
            .filter(|hit| hit.segment == index)
            .copied()
            .collect();
        hits.sort_by(|a, b| a.t.total_cmp(&b.t));
        let mut start = 0.0;
        let mut from = None;
        for hit in hits {
            if hit.t - start < 1e-6 {
                return None;
            }
            pieces.push((from, Some(hit.vertex), segment.subsegment(start..hit.t)));
            start = hit.t;
            from = Some(hit.vertex);
        }
        if 1.0 - start < 1e-6 {
            return None;
        }
        pieces.push((from, None, segment.subsegment(start..1.0)));
    }

    // Each boundary is a cycle; rotate it so the first piece begins at
    // a shared intersection. The closing cut falls at the last piece.
    let pivot = pieces.iter().position(|piece| piece.0.is_some())?;
    let mut from = pieces.get(pivot)?.0?;
    let mut active = Vec::new();
    let mut arcs = Vec::new();
    for i in 0..pieces.len() {
        let piece = pieces.get((pivot + i) % pieces.len())?;
        active.push(piece.2);
        if let Some(to) = piece.1 {
            let sample = active.get(active.len() / 2)?.eval(0.5);
            arcs.push(CurveEdge {
                from,
                to,
                inside_other: other.contains(sample),
                pieces: std::mem::take(&mut active),
            });
            from = to;
        }
    }
    if !active.is_empty() || arcs.len() != cuts.len() {
        return None;
    }
    Some(arcs)
}

fn reverse_edge(mut edge: CurveEdge) -> CurveEdge {
    std::mem::swap(&mut edge.from, &mut edge.to);
    edge.pieces = edge.pieces.into_iter().rev().map(|seg| seg.reverse()).collect();
    edge
}

/// Assemble directed arcs only if every visited crossing has exactly one
/// incoming and outgoing edge. Ambiguous topology uses the old solver.
fn crossing_cycles(edges: &[CurveEdge], intersections: usize) -> Option<Vec<BezPath>> {
    if edges.is_empty() {
        return None;
    }
    let mut next = vec![None; intersections];
    let mut incoming = vec![0usize; intersections];
    for (index, edge) in edges.iter().enumerate() {
        if edge.from >= intersections || edge.to >= intersections {
            return None;
        }
        let slot = next.get_mut(edge.from)?;
        if slot.replace(index).is_some() {
            return None;
        }
        let count = incoming.get_mut(edge.to)?;
        *count += 1;
        if *count > 1 {
            return None;
        }
    }
    if (0..intersections).any(|id| next[id].is_some() != (incoming[id] == 1)) {
        return None;
    }

    let mut used = vec![false; edges.len()];
    let mut shapes = Vec::new();
    for first in 0..edges.len() {
        if used[first] {
            continue;
        }
        let mut path = BezPath::new();
        path.move_to(edges.get(first)?.pieces.first()?.start());
        let mut current = first;
        let mut closed = false;
        for _ in 0..edges.len() {
            if used[current] {
                return None;
            }
            used[current] = true;
            let edge = edges.get(current)?;
            for segment in &edge.pieces {
                path.push(segment.as_path_el());
            }
            let following = *next.get(edge.to)?.as_ref()?;
            if following == first {
                path.close_path();
                closed = true;
                break;
            }
            current = following;
        }
        if !closed {
            return None;
        }
        shapes.push(path);
    }
    Some(shapes)
}

/// Bounds from the control polygon. A Bezier subsegment stays inside
/// the convex hull of its control points, including at non-monotone spans.
fn control_bounds(seg: &kurbo::PathSeg) -> (f64, f64, f64, f64) {
    let p = seg.start();
    let mut bounds = (p.x, p.y, p.x, p.y);
    let mut add = |q: Point| {
        bounds.0 = bounds.0.min(q.x);
        bounds.1 = bounds.1.min(q.y);
        bounds.2 = bounds.2.max(q.x);
        bounds.3 = bounds.3.max(q.y);
    };
    match seg {
        kurbo::PathSeg::Line(line) => add(line.p1),
        kurbo::PathSeg::Quad(quad) => {
            add(quad.p1);
            add(quad.p2);
        }
        kurbo::PathSeg::Cubic(cubic) => {
            add(cubic.p1);
            add(cubic.p2);
            add(cubic.p3);
        }
    }
    bounds
}

fn bounds_touch(
    a: (f64, f64, f64, f64),
    b: (f64, f64, f64, f64),
    gap: f64,
) -> bool {
    a.0 <= b.2 + gap && b.0 <= a.2 + gap
        && a.1 <= b.3 + gap && b.1 <= a.3 + gap
}

fn control_polygon_flat(seg: &kurbo::PathSeg, tol: f64) -> bool {
    let from = seg.start();
    let to = seg.end();
    let a = [from.x, from.y];
    let b = [to.x, to.y];
    let t2 = tol * tol;
    match seg {
        kurbo::PathSeg::Line(_) => true,
        kurbo::PathSeg::Quad(q) => {
            point_chord_distance_sq([q.p1.x, q.p1.y], a, b) <= t2
        }
        kurbo::PathSeg::Cubic(c) => {
            point_chord_distance_sq([c.p1.x, c.p1.y], a, b) <= t2
                && point_chord_distance_sq([c.p2.x, c.p2.y], a, b) <= t2
        }
    }
}

fn curve_velocity(seg: &kurbo::PathSeg, t: f64) -> kurbo::Vec2 {
    match seg {
        kurbo::PathSeg::Line(line) => line.p1 - line.p0,
        kurbo::PathSeg::Quad(quad) => {
            (quad.p1 - quad.p0) * (2.0 * (1.0 - t))
                + (quad.p2 - quad.p1) * (2.0 * t)
        }
        kurbo::PathSeg::Cubic(cubic) => {
            let u = 1.0 - t;
            (cubic.p1 - cubic.p0) * (3.0 * u * u)
                + (cubic.p2 - cubic.p1) * (6.0 * u * t)
                + (cubic.p3 - cubic.p2) * (3.0 * t * t)
        }
    }
}

fn cross_vectors(a: kurbo::Vec2, b: kurbo::Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// Refine a geometric chord crossing to the original parametric curves.
/// Reject near-tangencies and unstable Newton steps rather than silently
/// producing nodes with incorrect topology.
fn refine_curve_crossing(
    a: &kurbo::PathSeg,
    b: &kurbo::PathSeg,
    mut t: f64,
    mut u: f64,
) -> Option<(f64, f64)> {
    for _ in 0..20 {
        let delta = b.eval(u) - a.eval(t);
        let va = curve_velocity(a, t);
        let vb = curve_velocity(b, u);
        let la = va.hypot();
        let lb = vb.hypot();
        if !la.is_finite() || !lb.is_finite() || la < 1e-12 || lb < 1e-12 {
            return None;
        }
        let det = cross_vectors(va, vb);
        if det.abs() < 1e-4 * la * lb {
            return None;
        }
        if delta.hypot() < 1e-7 {
            return Some((t, u));
        }
        let dt = cross_vectors(delta, vb) / det;
        let du = cross_vectors(delta, va) / det;
        if !dt.is_finite() || !du.is_finite()
            || !(0.0..=1.0).contains(&(t + dt))
            || !(0.0..=1.0).contains(&(u + du))
        {
            return None;
        }
        t += dt;
        u += du;
    }
    ((a.eval(t) - b.eval(u)).hypot() < 1e-7).then_some((t, u))
}

#[derive(Clone)]
struct CurveWindow {
    segment: kurbo::PathSeg,
    low: f64,
    high: f64,
    depth: u8,
}

impl CurveWindow {
    fn halve(self) -> (Self, Self) {
        let middle = (self.low + self.high) * 0.5;
        let left = Self {
            segment: self.segment.subsegment(0.0..0.5),
            low: self.low,
            high: middle,
            depth: self.depth + 1,
        };
        let right = Self {
            segment: self.segment.subsegment(0.5..1.0),
            low: middle,
            high: self.high,
            depth: self.depth + 1,
        };
        (left, right)
    }
}

/// Isolate cubic/quad intersections with bounded subdivision of control
/// hulls. Only parameter values are approximated: the returned shape
/// pieces are still exact subsegments of the original curves. An
/// uncertain root, tangent, overlapping span or exhausted budget returns
/// None so the caller can use the established polygon algorithm.
fn curved_pair_roots(
    a: &kurbo::PathSeg,
    b: &kurbo::PathSeg,
) -> Option<Vec<(f64, f64)>> {
    const FLATNESS: f64 = 1e-7;
    const LIMIT: usize = 40_000;
    let mut pending = vec![(
        CurveWindow { segment: *a, low: 0.0, high: 1.0, depth: 0 },
        CurveWindow { segment: *b, low: 0.0, high: 1.0, depth: 0 },
    )];
    let mut roots: Vec<(f64, f64)> = Vec::new();
    let mut visited = 0usize;
    while let Some((left, right)) = pending.pop() {
        visited += 1;
        if visited > LIMIT {
            return None;
        }
        let a_bounds = control_bounds(&left.segment);
        let b_bounds = control_bounds(&right.segment);
        if !bounds_touch(a_bounds, b_bounds, 1e-9) {
            continue;
        }
        let a_flat = control_polygon_flat(&left.segment, FLATNESS);
        let b_flat = control_polygon_flat(&right.segment, FLATNESS);
        if a_flat && b_flat {
            let p = left.segment.start();
            let q = right.segment.start();
            let da = left.segment.end() - p;
            let db = right.segment.end() - q;
            let det = cross_vectors(da, db);
            let chord_len = da.hypot() * db.hypot();
            if chord_len <= 1e-22 {
                return None;
            }
            if det.abs() < 1e-4 * chord_len {
                // Reject potential coincident boundaries and tangencies.
                if (p - q).hypot() < 2e-6
                    || point_chord_distance_sq([p.x, p.y], [q.x, q.y],
                        [right.segment.end().x, right.segment.end().y]) < 4e-12
                {
                    return None;
                }
                continue;
            }
            let delta = q - p;
            let x = cross_vectors(delta, db) / det;
            let y = cross_vectors(delta, da) / det;
            if !(-1e-4..=1.0 + 1e-4).contains(&x)
                || !(-1e-4..=1.0 + 1e-4).contains(&y)
            {
                continue;
            }
            let ta = left.low + x.clamp(0.0, 1.0) * (left.high - left.low);
            let tb = right.low + y.clamp(0.0, 1.0) * (right.high - right.low);
            let (ta, tb) = refine_curve_crossing(a, b, ta, tb)?;
            if roots.iter().any(|(at, bt)|
                (*at - ta).abs() < 1e-6 && (*bt - tb).abs() < 1e-6
            ) {
                continue;
            }
            // Nearby distinct roots are numerically unsafe to sort/split.
            if roots.iter().any(|(at, bt)|
                (a.eval(*at) - a.eval(ta)).hypot() < 1e-5
                    || (b.eval(*bt) - b.eval(tb)).hypot() < 1e-5
            ) {
                return None;
            }
            roots.push((ta, tb));
            if roots.len() > 16 {
                return None;
            }
            continue;
        }
        if left.depth >= 28 || right.depth >= 28 {
            return None;
        }
        let ax = a_bounds.2 - a_bounds.0;
        let ay = a_bounds.3 - a_bounds.1;
        let bx = b_bounds.2 - b_bounds.0;
        let by = b_bounds.3 - b_bounds.1;
        if !a_flat && (b_flat || ax.max(ay) >= bx.max(by)) {
            let (first, second) = left.halve();
            pending.push((first, right.clone()));
            pending.push((second, right));
        } else {
            let (first, second) = right.halve();
            pending.push((left.clone(), first));
            pending.push((left, second));
        }
    }
    Some(roots)
}

/// Parametric solver for simple closed curve crossings. Both sides may
/// contain cubic or quadratic segments. A bounded curve/curve root solver
/// is used if neither segment is a line. Ambiguous topology, tangencies,
/// coincident spans and crossing XOR use the polygon fallback.
fn exact_curve_crossing(
    subject: &BezPath,
    clip: &BezPath,
    op: Op,
) -> Option<Vec<BezPath>> {
    use crate::geometry::{PathSeg, Shape as _};
    if op == Op::Exclude {
        return None;
    }
    let a_poly = simple_closed_contour(subject)?;
    let b_poly = simple_closed_contour(clip)?;
    let (a, a_closed) = subpath_segments(subject).pop()?;
    let (b, b_closed) = subpath_segments(clip).pop()?;
    if !a_closed || !b_closed || a.is_empty() || b.is_empty()
        || a.len() > 64 || b.len() > 64
    {
        return None;
    }

    let mut a_cuts = Vec::new();
    let mut b_cuts = Vec::new();
    let mut vertices: Vec<Point> = Vec::new();
    for (i, sa) in a.iter().enumerate() {
        for (j, sb) in b.iter().enumerate() {
            let hits: Vec<(f64, f64)> = match (sa, sb) {
                (PathSeg::Line(line), _) => sb
                    .intersect_line(*line)
                    .iter()
                    .map(|h| (h.line_t, h.segment_t))
                    .collect(),
                (_, PathSeg::Line(line)) => sa
                    .intersect_line(*line)
                    .iter()
                    .map(|h| (h.segment_t, h.line_t))
                    .collect(),
                _ => curved_pair_roots(sa, sb)?,
            };
            for (ta, tb) in hits {
                // Do not guess ownership of a vertex shared by segments.
                if !ta.is_finite() || !tb.is_finite()
                    || !(1e-5..=1.0 - 1e-5).contains(&ta)
                    || !(1e-5..=1.0 - 1e-5).contains(&tb)
                {
                    return None;
                }
                let pa = sa.eval(ta);
                let pb = sb.eval(tb);
                if (pa - pb).hypot() > 1e-6
                    || vertices.iter().any(|p| (*p - pa).hypot() < 1e-5)
                {
                    return None;
                }
                // A tangent contact is not a topological crossing.
                let delta = 1e-5;
                let da = sa.eval(ta + delta) - sa.eval(ta - delta);
                let db = sb.eval(tb + delta) - sb.eval(tb - delta);
                let cross = da.x * db.y - da.y * db.x;
                if cross.abs() < 1e-4 * da.hypot() * db.hypot()
                    || da.hypot() < 1e-10
                    || db.hypot() < 1e-10
                {
                    return None;
                }
                let id = vertices.len();
                vertices.push(pa);
                a_cuts.push(CurveCut { segment: i, t: ta, vertex: id });
                b_cuts.push(CurveCut { segment: j, t: tb, vertex: id });
                if vertices.len() > 32 {
                    return None;
                }
            }
        }
    }
    if vertices.len() < 2 || vertices.len() % 2 != 0 {
        return None;
    }
    let a_arcs = crossing_arcs(&a, &a_cuts, clip)?;
    let b_arcs = crossing_arcs(&b, &b_cuts, subject)?;
    let mut kept = Vec::new();
    for edge in a_arcs {
        let include = match op {
            Op::Weld | Op::Trim => !edge.inside_other,
            Op::Intersect => edge.inside_other,
            Op::Exclude => false,
        };
        if include {
            kept.push(edge);
        }
    }

    let same_winding = signed_polygon_area(&a_poly).signum()
        == signed_polygon_area(&b_poly).signum();
    for edge in b_arcs {
        let include = match op {
            Op::Weld => !edge.inside_other,
            Op::Intersect | Op::Trim => edge.inside_other,
            Op::Exclude => false,
        };
        if include {
            let forward = if op == Op::Trim { !same_winding } else { same_winding };
            kept.push(if forward { edge } else { reverse_edge(edge) });
        }
    }
    let paths = crossing_cycles(&kept, vertices.len())?;

    // Last conservative guard: the stitched paths must agree with the
    // expected boolean membership at representative interior samples.
    // Ambiguous or incorrectly oriented output falls back to polygons.
    let result = join_exact_paths(&paths);
    let sa = subject.bounding_box();
    let sb = clip.bounding_box();
    let x0 = sa.x0.min(sb.x0);
    let y0 = sa.y0.min(sb.y0);
    let width = sa.x1.max(sb.x1) - x0;
    let height = sa.y1.max(sb.y1) - y0;
    if width <= 0.0 || height <= 0.0 || !width.is_finite() || !height.is_finite() {
        return None;
    }
    for ix in 1..=7 {
        for iy in 1..=7 {
            let point = Point::new(
                x0 + width * (ix as f64 / 8.0),
                y0 + height * (iy as f64 / 8.0),
            );
            let in_a = subject.contains(point);
            let in_b = clip.contains(point);
            let expected = match op {
                Op::Weld => in_a || in_b,
                Op::Intersect => in_a && in_b,
                Op::Trim => in_a && !in_b,
                Op::Exclude => return None,
            };
            if result.contains(point) != expected {
                return None;
            }
        }
    }
    Some(paths)
}

/// Combine `subject` and `clip` (both in the same coordinate space).
pub fn overlay(subject: &BezPath, clip: &BezPath, op: Op) -> BezPath {
    if let Some(parts) = exact_non_crossing_parts(subject, clip, op)
        .or_else(|| exact_curve_crossing(subject, clip, op))
    {
        return join_exact_paths(&parts);
    }
    let subj = contours(subject);
    let clp = contours(clip);
    let rule = match op {
        Op::Weld => OverlayRule::Union,
        Op::Intersect => OverlayRule::Intersect,
        Op::Trim => OverlayRule::Difference,
        Op::Exclude => OverlayRule::Xor,
    };
    let result = subj.overlay(&clp, rule, IFill::EvenOdd);
    from_shapes(result)
}

/// Combine `subject` and `clip` as [`overlay`] does, each separate piece
/// of the result (its outline with its holes) a path of its own.
pub fn overlay_parts(subject: &BezPath, clip: &BezPath, op: Op) -> Vec<BezPath> {
    if let Some(parts) = exact_non_crossing_parts(subject, clip, op)
        .or_else(|| exact_curve_crossing(subject, clip, op))
    {
        return parts;
    }
    let subj = contours(subject);
    let clp = contours(clip);
    let rule = match op {
        Op::Weld => OverlayRule::Union,
        Op::Intersect => OverlayRule::Intersect,
        Op::Trim => OverlayRule::Difference,
        Op::Exclude => OverlayRule::Xor,
    };
    subj.overlay(&clp, rule, IFill::EvenOdd)
        .into_iter()
        .map(|shape| from_shapes(vec![shape]))
        .filter(|p| !p.elements().is_empty())
        .collect()
}

/// How an offset outline turns its corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    /// Sharp, the sides run on until they meet (very sharp corners, under
    /// about 9 degrees, are cut off).
    #[default]
    Miter,
    Round,
    /// Cut off straight.
    Bevel,
}

/// The smallest corner a mitred join keeps sharp, radians.
const MITER_MIN: f64 = 0.15;

/// Offset a closed path outward (positive) or inward (negative) by `d` mm,
/// with round corners. Used by Contour and outline-to-object.
pub fn offset(path: &BezPath, d: f64) -> BezPath {
    offset_with(path, d, Join::Round)
}

/// Offset a closed path outward (positive) or inward (negative) by `d` mm,
/// the corners turned as `join` says.
pub fn offset_with(path: &BezPath, d: f64, join: Join) -> BezPath {
    use i_overlay::float::simplify::SimplifyShape;
    use i_overlay::mesh::float::outline::offset::OutlineOffset;
    use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
    if !d.is_finite() {
        return BezPath::new();
    }
    let subj = contours(path);
    // Simplify guarantees orientation (outer CCW, holes CW) and no self-intersections.
    let clean = subj.simplify_shape(IFill::EvenOdd);
    let style = OutlineStyle::new(d).line_join(match join {
        Join::Miter => LineJoin::Miter(MITER_MIN),
        Join::Round => LineJoin::Round(0.05),
        Join::Bevel => LineJoin::Bevel,
    });
    let out = clean.outline(&style);
    from_shapes(out)
}

/// The area swept by a round or square nib of width `width` along a
/// polyline (the Eraser's band). A single point gives a dot.
pub fn stroke_band(points: &[Point], width: f64, square: bool) -> BezPath {
    use crate::geometry::Shape as _;
    use i_overlay::mesh::float::stroke::offset::StrokeOffset;
    use i_overlay::mesh::float::style::{LineCap, LineJoin, StrokeStyle};
    if points.is_empty() || width <= 0.0 {
        return BezPath::new();
    }
    let r = width / 2.0;
    if points.len() == 1 || points.iter().all(|p| (*p - points[0]).hypot() < 1e-9) {
        let c = points[0];
        return if square {
            crate::geometry::Rect::new(c.x - r, c.y - r, c.x + r, c.y + r).to_path(0.01)
        } else {
            crate::geometry::Circle::new(c, r).to_path(0.01)
        };
    }
    let pts: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    let style = if square {
        StrokeStyle::new(width)
            .line_join(LineJoin::Miter(MITER_MIN))
            .start_cap(LineCap::Square)
            .end_cap(LineCap::Square)
    } else {
        StrokeStyle::new(width)
            .line_join(LineJoin::Round(0.05))
            .start_cap(LineCap::Round(0.05))
            .end_cap(LineCap::Round(0.05))
    };
    from_shapes(pts.stroke(style, false))
}

/// Area swept by a calligraphic nib along a path: an ellipse of width
/// `width` along `nib_angle_deg` and `width * stretch` across it. Each
/// flattened segment contributes the parallelogram the nib's support
/// sweeps, each vertex the nib itself; the union of them is the outline
/// (open subpaths get nib-shaped ends). `stretch` 1 is the round pen.
pub fn calligraphic_band(path: &BezPath, width: f64, stretch: f64, nib_angle_deg: f64) -> BezPath {
    if width <= 0.0 {
        return BezPath::new();
    }
    let a = width / 2.0;
    let b = a * stretch.clamp(0.01, 1.0);
    let ang = nib_angle_deg.to_radians();
    let e1 = [ang.cos(), ang.sin()];
    let e2 = [-ang.sin(), ang.cos()];
    // Polylines of every subpath, closed ones with the first point repeated.
    let mut lines: Vec<Vec<Point>> = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), 0.05, &mut |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() >= 2 {
                lines.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur.push(p);
        }
        PathEl::LineTo(p) => cur.push(p),
        PathEl::ClosePath => {
            if let Some(f) = cur.first().copied() {
                cur.push(f);
            }
        }
        _ => {}
    });
    if !cur.is_empty() {
        lines.push(cur);
    }
    let ellipse = |c: Point| -> Vec<[f64; 2]> {
        (0..24)
            .map(|i| {
                let t = i as f64 / 24.0 * std::f64::consts::TAU;
                let (x, y) = (a * t.cos(), b * t.sin());
                [c.x + x * e1[0] + y * e2[0], c.y + x * e1[1] + y * e2[1]]
            })
            .collect()
    };
    let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
    for line in &lines {
        for (i, p) in line.iter().enumerate() {
            polys.push(ellipse(*p));
            let Some(q) = line.get(i + 1) else {
                continue;
            };
            let d = *q - *p;
            let len = d.hypot();
            if len < 1e-9 {
                continue;
            }
            let n = [-d.y / len, d.x / len];
            // Support half-width of the ellipse along n.
            let h = ((a * (n[0] * e1[0] + n[1] * e1[1])).powi(2)
                + (b * (n[0] * e2[0] + n[1] * e2[1])).powi(2))
            .sqrt();
            if h < 1e-9 {
                continue;
            }
            polys.push(vec![
                [p.x + n[0] * h, p.y + n[1] * h],
                [q.x + n[0] * h, q.y + n[1] * h],
                [q.x - n[0] * h, q.y - n[1] * h],
                [p.x - n[0] * h, p.y - n[1] * h],
            ]);
        }
    }
    // Consistent orientation so the non-zero union adds up.
    for poly in polys.iter_mut() {
        let area: f64 = poly
            .iter()
            .zip(poly.iter().cycle().skip(1))
            .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
            .sum();
        if area < 0.0 {
            poly.reverse();
        }
    }
    let empty: Vec<Vec<[f64; 2]>> = Vec::new();
    from_shapes(polys.overlay(&empty, OverlayRule::Subject, IFill::NonZero))
}

/// The polylines of every subpath (flattened to `tol`), closed ones with
/// the first point repeated at the end.
fn polylines(path: &BezPath, tol: f64) -> Vec<(Vec<Point>, bool)> {
    let mut lines: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), tol, &mut |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() >= 2 {
                lines.push((std::mem::take(&mut cur), false));
            } else {
                cur.clear();
            }
            cur.push(p);
        }
        PathEl::LineTo(p) => cur.push(p),
        PathEl::ClosePath => {
            if let Some(f) = cur.first().copied() {
                cur.push(f);
            }
            if cur.len() >= 2 {
                lines.push((std::mem::take(&mut cur), true));
            } else {
                cur.clear();
            }
        }
        _ => {}
    });
    if cur.len() >= 2 {
        lines.push((cur, false));
    }
    lines
}

/// Total length of a path's flattened subpaths.
pub fn path_length(path: &BezPath) -> f64 {
    polylines(path, 0.05)
        .iter()
        .map(|(l, _)| l.windows(2).map(|w| (w[1] - w[0]).hypot()).sum::<f64>())
        .sum()
}

/// Where along the path (0 to 1, by length over all subpaths) the point
/// nearest to `p` is, its distance and which side of the path's direction
/// `p` is on (true: left).
pub fn locate_on_path(path: &BezPath, p: Point) -> Option<(f64, f64, bool)> {
    let lines = polylines(path, 0.05);
    let total: f64 = lines
        .iter()
        .map(|(l, _)| l.windows(2).map(|w| (w[1] - w[0]).hypot()).sum::<f64>())
        .sum();
    if total <= 1e-12 {
        return None;
    }
    let mut best: Option<(f64, f64, bool)> = None;
    let mut run = 0.0;
    for (line, _) in &lines {
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = b - a;
            let len2 = d.x * d.x + d.y * d.y;
            if len2 <= 1e-18 {
                continue;
            }
            let len = len2.sqrt();
            let u = (((p - a).x * d.x + (p - a).y * d.y) / len2).clamp(0.0, 1.0);
            let q = a + d * u;
            let dist = (p - q).hypot();
            if best.map(|(_, bd, _)| dist < bd).unwrap_or(true) {
                let cross = d.x * (p - a).y - d.y * (p - a).x;
                best = Some(((run + u * len) / total, dist, cross >= 0.0));
            }
            run += len;
        }
    }
    best
}

/// The point `t` of the way along the path (0 to 1, by length over all
/// subpaths) and the unit normal to the left of its direction there.
pub fn point_on_path(path: &BezPath, t: f64) -> Option<(Point, [f64; 2])> {
    let lines = polylines(path, 0.05);
    let total: f64 = lines
        .iter()
        .map(|(l, _)| l.windows(2).map(|w| (w[1] - w[0]).hypot()).sum::<f64>())
        .sum();
    if total <= 1e-12 {
        return None;
    }
    let want = t.clamp(0.0, 1.0) * total;
    let mut run = 0.0;
    let mut last = None;
    for (line, _) in &lines {
        for w in line.windows(2) {
            let d = w[1] - w[0];
            let len = d.hypot();
            if len <= 1e-12 {
                continue;
            }
            let n = [-d.y / len, d.x / len];
            if run + len >= want {
                let u = ((want - run) / len).clamp(0.0, 1.0);
                return Some((w[0] + d * u, n));
            }
            run += len;
            last = Some((w[1], n));
        }
    }
    last
}

/// The area of a variable-width outline: every segment of the flattened
/// path widened by the reach `width_at` gives at its ends (left and right
/// separately), joined with wedges at the corners; butt ends.
pub fn variable_band(path: &BezPath, base: f64, nodes: &[crate::style::WidthNode]) -> BezPath {
    let lines = polylines(path, 0.05);
    let total: f64 = lines
        .iter()
        .map(|(l, _)| l.windows(2).map(|w| (w[1] - w[0]).hypot()).sum::<f64>())
        .sum();
    if total <= 1e-12 {
        return BezPath::new();
    }
    let reach = |s: f64| crate::style::width_at(base, nodes, s / total);
    // Short pieces so the width can change along long straight segments,
    // split at the nodes so their reach is met exactly.
    let step = (total / 256.0).max(0.05);
    let mut marks: Vec<f64> = nodes.iter().map(|n| n.at.clamp(0.0, 1.0) * total).collect();
    marks.sort_by(|a, b| a.total_cmp(b));
    let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut run = 0.0;
    for (line, closed) in &lines {
        // Resampled points with their length along the whole path.
        let mut pts: Vec<(Point, f64)> = Vec::new();
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (b - a).hypot();
            if pts.is_empty() {
                pts.push((a, run));
            }
            if len <= 1e-12 {
                continue;
            }
            let k = ((len / step).ceil() as usize).clamp(1, 4096);
            let mut cuts: Vec<f64> = (1..k).map(|i| i as f64 / k as f64).collect();
            cuts.extend(
                marks
                    .iter()
                    .filter(|m| **m > run + 1e-9 && **m < run + len - 1e-9)
                    .map(|m| (m - run) / len),
            );
            cuts.sort_by(|x, y| x.total_cmp(y));
            for u in cuts {
                pts.push((a + (b - a) * u, run + u * len));
            }
            run += len;
            pts.push((b, run));
        }
        // Unit normals (to the left) of each piece.
        let normals: Vec<Option<[f64; 2]>> = pts
            .windows(2)
            .map(|w| {
                let d = w[1].0 - w[0].0;
                let len = d.hypot();
                (len > 1e-12).then(|| [-d.y / len, d.x / len])
            })
            .collect();
        for (i, w) in pts.windows(2).enumerate() {
            let Some(n) = normals.get(i).copied().flatten() else {
                continue;
            };
            let ((p, s0), (q, s1)) = (w[0], w[1]);
            let (l0, r0) = reach(s0);
            let (l1, r1) = reach(s1);
            polys.push(vec![
                [p.x + n[0] * l0, p.y + n[1] * l0],
                [q.x + n[0] * l1, q.y + n[1] * l1],
                [q.x - n[0] * r1, q.y - n[1] * r1],
                [p.x - n[0] * r0, p.y - n[1] * r0],
            ]);
        }
        // Round joins between consecutive pieces (and around the seam of
        // a closed subpath): a fan from one normal to the next on each
        // side.
        let valid: Vec<(usize, [f64; 2])> = normals
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| (i, n)))
            .collect();
        let count = valid.len();
        let pairs = if *closed {
            count
        } else {
            count.saturating_sub(1)
        };
        for k in 0..pairs {
            let (Some(&(i, a)), Some(&(_, b))) = (valid.get(k), valid.get((k + 1) % count.max(1)))
            else {
                continue;
            };
            let Some(&(c, sc)) = pts.get(i + 1) else {
                continue;
            };
            let turn = (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]);
            if turn.abs() < 1e-6 {
                continue;
            }
            let (l, r) = reach(sc);
            let a0 = a[1].atan2(a[0]);
            let steps = ((turn.abs() / 0.2).ceil() as usize).clamp(1, 32);
            for (sign, len) in [(1.0, l), (-1.0, r)] {
                if len <= 0.0 {
                    continue;
                }
                let mut fan = vec![[c.x, c.y]];
                for j in 0..=steps {
                    let ang = a0 + turn * j as f64 / steps as f64;
                    fan.push([c.x + sign * ang.cos() * len, c.y + sign * ang.sin() * len]);
                }
                polys.push(fan);
            }
        }
    }
    for poly in polys.iter_mut() {
        let area: f64 = poly
            .iter()
            .zip(poly.iter().cycle().skip(1))
            .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
            .sum();
        if area < 0.0 {
            poly.reverse();
        }
    }
    polys.retain(|p| {
        p.iter()
            .zip(p.iter().cycle().skip(1))
            .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
            .sum::<f64>()
            .abs()
            > 1e-12
    });
    let empty: Vec<Vec<[f64; 2]>> = Vec::new();
    from_shapes(polys.overlay(&empty, OverlayRule::Subject, IFill::NonZero))
}

/// The filled area that stands for an outline that cannot be drawn as a
/// plain stroke (a calligraphic nib, a variable width, an outline inside
/// or outside the path, or dashes fitted to the corners), for a path in
/// page space and the object's scale (1 when the outline does not scale
/// with the object). `None`: a plain stroke draws it.
pub fn outline_band(path: &BezPath, stroke: &crate::style::Stroke, scale: f64) -> Option<BezPath> {
    use crate::style::{OutlinePosition, Stroke};
    if !stroke.widths.is_empty() {
        let nodes: Vec<crate::style::WidthNode> = stroke
            .widths
            .iter()
            .map(|n| crate::style::WidthNode {
                at: n.at,
                left: n.left * scale,
                right: n.right * scale,
            })
            .collect();
        return Some(variable_band(path, stroke.width * scale, &nodes));
    }
    let hair = stroke.width <= Stroke::HAIRLINE + 1e-9;
    if stroke.stretch < 0.999 && !hair {
        return Some(calligraphic_band(
            path,
            stroke.width * scale,
            stroke.stretch,
            stroke.nib_angle,
        ));
    }
    if hair || !stroke.width.is_finite() {
        return None;
    }
    let scaled = Stroke {
        width: stroke.width * scale,
        ..stroke.clone()
    };
    let aligned = stroke.align_dashes && has_dashes(&stroke.dash);
    if stroke.position == OutlinePosition::Center {
        return aligned.then(|| stroke_area(path, &scaled));
    }
    Some(positioned_band(path, &scaled))
}

/// True when a dash pattern has something to draw.
fn has_dashes(dash: &[f64]) -> bool {
    !dash.is_empty() && dash.iter().all(|d| d.is_finite()) && dash.iter().sum::<f64>() > 1e-9
}

/// An outline wholly inside or outside the closed subpaths of `path`
/// (page space, width already scaled): the stroke at twice the width,
/// cut to the fill area (inside) or with the fill area cut away
/// (outside). Open subpaths keep a centred outline. Dash lengths stay
/// those of the outline's own width.
fn positioned_band(path: &BezPath, stroke: &crate::style::Stroke) -> BezPath {
    use crate::style::OutlinePosition;
    let (closed, open) = split_closed(path);
    let mut out = BezPath::new();
    if !closed.elements().is_empty() {
        let doubled = crate::style::Stroke {
            width: stroke.width * 2.0,
            dash: stroke.dash.iter().map(|d| d / 2.0).collect(),
            ..stroke.clone()
        };
        let area = simplify(&stroke_area(&closed, &doubled));
        let fill = simplify_even_odd(&closed);
        let op = match stroke.position {
            OutlinePosition::Inside => Op::Intersect,
            _ => Op::Trim,
        };
        out = overlay(&area, &fill, op);
    }
    if !open.elements().is_empty() {
        let centred = simplify(&stroke_area(&open, stroke));
        out = if out.elements().is_empty() {
            centred
        } else {
            overlay(&out, &centred, Op::Weld)
        };
    }
    out
}

/// Split a path into its closed subpaths (ended with a close command or
/// ending where they start) and its open ones.
pub fn split_closed(path: &BezPath) -> (BezPath, BezPath) {
    let mut closed = BezPath::new();
    let mut open = BezPath::new();
    let mut cur: Vec<PathEl> = Vec::new();
    let mut flush = |cur: &mut Vec<PathEl>| {
        if cur.is_empty() {
            return;
        }
        let start = match cur.first() {
            Some(PathEl::MoveTo(p)) => Some(*p),
            _ => None,
        };
        let end = match cur.last() {
            Some(PathEl::LineTo(p))
            | Some(PathEl::QuadTo(_, p))
            | Some(PathEl::CurveTo(_, _, p)) => Some(*p),
            _ => None,
        };
        let is_closed = cur.iter().any(|e| matches!(e, PathEl::ClosePath))
            || matches!((start, end), (Some(a), Some(b)) if (a - b).hypot() < 1e-6);
        let target = if is_closed { &mut closed } else { &mut open };
        for e in cur.drain(..) {
            target.push(e);
        }
    };
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            flush(&mut cur);
        }
        cur.push(*el);
    }
    flush(&mut cur);
    (closed, open)
}

/// The area of a path filled with the even-odd rule, as clean contours.
fn simplify_even_odd(path: &BezPath) -> BezPath {
    let subj = contours(path);
    let empty: Vec<Vec<[f64; 2]>> = Vec::new();
    from_shapes(subj.overlay(&empty, OverlayRule::Subject, IFill::EvenOdd))
}

/// The area a plain outline covers along `path` (page space), as a path
/// to fill with the non-zero rule: its width, caps, joins (with its mitre
/// limit) and dashes, fitted to the corners when it aligns them. A
/// hairline covers no area: the result is empty.
pub fn stroke_area(path: &BezPath, stroke: &crate::style::Stroke) -> BezPath {
    use kurbo::{Cap, Join};
    if !stroke.width.is_finite() || stroke.width <= crate::style::Stroke::HAIRLINE + 1e-9 {
        return BezPath::new();
    }
    let cap = match stroke.cap {
        crate::style::LineCap::Butt => Cap::Butt,
        crate::style::LineCap::Round => Cap::Round,
        crate::style::LineCap::Square => Cap::Square,
    };
    let join = match stroke.join {
        crate::style::LineJoin::Miter => Join::Miter,
        crate::style::LineJoin::Round => Join::Round,
        crate::style::LineJoin::Bevel => Join::Bevel,
    };
    let mut style = kurbo::Stroke::new(stroke.width)
        .with_caps(cap)
        .with_join(join)
        .with_miter_limit(stroke.miter_ratio());
    let dashes: Vec<f64> = stroke
        .dash
        .iter()
        .map(|d| (d * stroke.width).max(0.0))
        .collect();
    let tol = (stroke.width / 100.0).max(1e-3);
    if has_dashes(&dashes) {
        if stroke.align_dashes {
            let pieces = aligned_dashes(path, &dashes);
            return kurbo::stroke(pieces.iter(), &style, &kurbo::StrokeOpts::default(), tol);
        }
        style = style.with_dashes(0.0, dashes);
    }
    kurbo::stroke(path.iter(), &style, &kurbo::StrokeOpts::default(), tol)
}

/// Corners sharper than this (degrees between the tangents) split a path
/// into runs for dash alignment; smooth nodes do not.
const DASH_CORNER_DEG: f64 = 3.0;

/// The dashes of `path` with the pattern `dashes` (on, off, on, ...
/// lengths in mm) fitted to each run between corners: a run of length
/// `L` takes `n = max(1, round(L / P))` repeats of the pattern (sum `P`)
/// stretched by `L / (n P)`, starting and ending with half of the first
/// dash. Halves meeting at a corner or at the start of a closed subpath
/// join into one dash, so every corner and every line end lies inside a
/// dash. The result is the dashes as open subpaths, to be stroked
/// without a pattern.
pub fn aligned_dashes(path: &BezPath, dashes: &[f64]) -> BezPath {
    let mut pattern: Vec<f64> = dashes.iter().map(|d| d.max(0.0)).collect();
    if pattern.len() % 2 == 1 {
        pattern = [pattern.clone(), pattern].concat();
    }
    let period: f64 = pattern.iter().sum();
    let mut out = BezPath::new();
    if pattern.is_empty() || !(period.is_finite() && period > 1e-9) {
        return path.clone();
    }
    for (segs, closed) in subpath_segments(path) {
        let lens: Vec<f64> = segs.iter().map(|s| s.arclen(1e-4)).collect();
        let total: f64 = lens.iter().sum();
        if total <= 1e-9 {
            continue;
        }
        // Run boundaries (arc length) at the corners.
        let mut cuts = vec![0.0];
        let mut acc = 0.0;
        for (i, pair) in segs.windows(2).enumerate() {
            acc += lens[i];
            if is_corner(&pair[0], &pair[1]) {
                cuts.push(acc);
            }
        }
        cuts.push(total);
        // A closed subpath smooth where it starts runs on across the start.
        let wraps = closed && !segs.last().is_some_and(|l| is_corner(l, &segs[0]));
        let mut runs: Vec<(f64, f64)> = cuts.windows(2).map(|w| (w[0], w[1])).collect();
        if wraps && runs.len() > 1 {
            // The last run continues into the first.
            let (a, _) = runs.remove(runs.len() - 1);
            let (_, b) = runs[0];
            runs[0] = (a, total + b);
        }
        let mut intervals: Vec<(f64, f64)> = Vec::new();
        for (r0, r1) in runs {
            let len = r1 - r0;
            if len <= 1e-9 {
                continue;
            }
            let n = (len / period).round().max(1.0);
            let k = len / (n * period);
            let q: Vec<f64> = pattern.iter().map(|d| (d * k).max(1e-3)).collect();
            let p: f64 = q.iter().sum();
            let half = q[0] / 2.0;
            for rep in 0..=(n as usize) {
                let mut off = rep as f64 * p - half;
                for (i, d) in q.iter().enumerate() {
                    if i % 2 == 0 {
                        let a = off.max(0.0);
                        let b = (off + d).min(len);
                        if b > a + 1e-9 {
                            intervals.push((r0 + a, r0 + b));
                        }
                    }
                    off += d;
                }
            }
        }
        // Dashes of a run that crossed the start, measured from the start.
        for iv in intervals.iter_mut() {
            if iv.0 >= total - 1e-9 {
                *iv = (iv.0 - total, iv.1 - total);
            }
        }
        // Join dashes that meet (the halves at a corner).
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (a, b) in intervals {
            match merged.last_mut() {
                Some(last) if a <= last.1 + 1e-6 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        // A closed subpath: the dash ending at the start runs on into the
        // first one.
        if closed && merged.len() > 1 {
            let first = merged[0];
            if let Some(last) = merged.last().copied() {
                if first.0 <= 1e-6 && (last.1 - total).abs() <= 1e-6 {
                    merged.pop();
                    merged[0] = (last.0, total + first.1);
                }
            }
        }
        for (a, b) in merged {
            append_piece(&mut out, &segs, &lens, total, closed, a, b);
        }
    }
    out
}

/// The segments of each subpath of `path` (a close adds the closing
/// line), with whether it is closed. Zero-length segments are left out.
fn subpath_segments(path: &BezPath) -> Vec<(Vec<kurbo::PathSeg>, bool)> {
    use kurbo::{CubicBez, Line, PathSeg, QuadBez};
    let mut out = Vec::new();
    let mut segs: Vec<PathSeg> = Vec::new();
    let mut start = Point::ZERO;
    let mut cur = Point::ZERO;
    let push = |segs: &mut Vec<PathSeg>, s: PathSeg| {
        if s.arclen(1e-6) > 1e-9 {
            segs.push(s);
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !segs.is_empty() {
                    out.push((std::mem::take(&mut segs), false));
                }
                start = p;
                cur = p;
            }
            PathEl::LineTo(p) => {
                push(&mut segs, PathSeg::Line(Line::new(cur, p)));
                cur = p;
            }
            PathEl::QuadTo(a, p) => {
                push(&mut segs, PathSeg::Quad(QuadBez::new(cur, a, p)));
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push(&mut segs, PathSeg::Cubic(CubicBez::new(cur, a, b, p)));
                cur = p;
            }
            PathEl::ClosePath => {
                push(&mut segs, PathSeg::Line(Line::new(cur, start)));
                cur = start;
                if !segs.is_empty() {
                    out.push((std::mem::take(&mut segs), true));
                }
            }
        }
    }
    if !segs.is_empty() {
        out.push((segs, false));
    }
    out
}

/// True when the path turns by more than [`DASH_CORNER_DEG`] where `a`
/// ends and `b` starts.
fn is_corner(a: &kurbo::PathSeg, b: &kurbo::PathSeg) -> bool {
    use kurbo::ParamCurve;
    let ta = a.eval(1.0) - a.eval(1.0 - 1e-3);
    let tb = b.eval(1e-3) - b.eval(0.0);
    let (la, lb) = (ta.hypot(), tb.hypot());
    if la <= 1e-12 || lb <= 1e-12 {
        return true;
    }
    let cos = (ta.dot(tb) / (la * lb)).clamp(-1.0, 1.0);
    cos.acos().to_degrees() > DASH_CORNER_DEG
}

/// Append the part of a subpath between arc lengths `a` and `b` as an open
/// subpath; on a closed subpath `b` may run past the end into the start.
fn append_piece(
    out: &mut BezPath,
    segs: &[kurbo::PathSeg],
    lens: &[f64],
    total: f64,
    closed: bool,
    a: f64,
    b: f64,
) {
    let mut ranges = vec![(a, b.min(total))];
    if closed && b > total + 1e-9 {
        ranges.push((0.0, b - total));
    }
    let mut started = false;
    for (ra, rb) in ranges {
        let mut s0 = 0.0;
        for (seg, len) in segs.iter().zip(lens) {
            let s1 = s0 + len;
            let lo = ra.max(s0);
            let hi = rb.min(s1);
            if hi > lo + 1e-9 && *len > 1e-12 {
                let t0 = seg.inv_arclen(lo - s0, 1e-5);
                let t1 = seg.inv_arclen(hi - s0, 1e-5);
                let piece = seg.subsegment(t0..t1);
                if !started {
                    out.move_to(piece.start());
                    started = true;
                }
                match piece {
                    kurbo::PathSeg::Line(l) => out.line_to(l.p1),
                    kurbo::PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                    kurbo::PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
                }
            }
            s0 = s1;
        }
    }
}

/// Union of one path with itself: removes self-overlaps (Simplify).
pub fn simplify(path: &BezPath) -> BezPath {
    // A simple closed contour has no overlaps to remove. Preserve its
    // original editable lines, quadratics and cubics without flattening.
    if simple_closed_contour(path).is_some() {
        return path.clone();
    }
    let subj = contours(path);
    let empty: Vec<Vec<[f64; 2]>> = Vec::new();
    let result = subj.overlay(&empty, OverlayRule::Subject, IFill::NonZero);
    from_shapes(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn variable_band_follows_the_width_nodes() {
        use super::{locate_on_path, outline_band, variable_band};
        use crate::geometry::{BezPath, Point, Shape as _};
        use crate::style::{Stroke, WidthNode};
        let mut line = BezPath::new();
        line.move_to(Point::new(0.0, 0.0));
        line.line_to(Point::new(100.0, 0.0));
        // No nodes: the base width everywhere, butt ends.
        let b = variable_band(&line, 2.0, &[]).bounding_box();
        assert!((b.height() - 2.0).abs() < 1e-6 && (b.width() - 100.0).abs() < 1e-6);
        // A 10 mm node in the middle: 10 mm there, 2 mm at the ends.
        let band = variable_band(&line, 2.0, &[WidthNode::symmetric(0.5, 10.0)]);
        let b = band.bounding_box();
        assert!((b.height() - 10.0).abs() < 1e-3, "{b:?}");
        assert!(band.contains(Point::new(50.0, 4.9)));
        assert!(!band.contains(Point::new(2.0, 1.5)));
        assert!(band.contains(Point::new(2.0, 0.9)));
        // Only to the left (up, for a line going right).
        let node = WidthNode {
            at: 0.5,
            left: 6.0,
            right: 0.0,
        };
        let b = variable_band(&line, 0.0, &[node]).bounding_box();
        assert!(b.y0 > -1e-6 && (b.y1 - 6.0).abs() < 1e-3, "{b:?}");
        // Through the stroke, scaled with the object.
        let mut s = Stroke::new(crate::color::Color::BLACK, 2.0);
        assert!(outline_band(&line, &s, 1.0).is_none());
        s.widths.push(WidthNode::symmetric(0.5, 10.0));
        let b = outline_band(&line, &s, 2.0)
            .map(|p| p.bounding_box())
            .unwrap_or_default();
        assert!((b.height() - 20.0).abs() < 1e-3, "{b:?}");
        // Locating: a quarter along, 3 mm to the right.
        let (t, d, left) = locate_on_path(&line, Point::new(25.0, -3.0)).unwrap_or_default();
        assert!((t - 0.25).abs() < 1e-9 && (d - 3.0).abs() < 1e-9 && !left);
    }

    #[test]
    fn closed_variable_bands_have_no_gaps_at_the_corners() {
        use super::variable_band;
        use crate::geometry::{Point, Rect, Shape as _};
        let sq = Rect::new(0.0, 0.0, 50.0, 50.0).to_path(0.01);
        let band = variable_band(&sq, 4.0, &[]);
        // The outer corner is covered on both sides of the seam.
        assert!(band.contains(Point::new(-1.2, -1.2)));
        assert!(band.contains(Point::new(51.2, 51.2)));
        assert!(!band.contains(Point::new(-1.6, -1.6)));
        assert!(!band.contains(Point::new(25.0, 25.0)));
    }

    #[test]
    fn calligraphic_band_is_thin_along_the_nib_and_full_across_it() {
        use super::calligraphic_band;
        use crate::geometry::{BezPath, Point, Shape as _};
        // Flat nib at 0 degrees, 2 mm wide, stretch 0.1.
        let mut horizontal = BezPath::new();
        horizontal.move_to(Point::new(0.0, 0.0));
        horizontal.line_to(Point::new(50.0, 0.0));
        let hb = calligraphic_band(&horizontal, 2.0, 0.1, 0.0).bounding_box();
        // Across a horizontal stroke the nib offers only its 0.2 mm side.
        assert!(hb.height() < 0.25 && hb.height() > 0.15, "{hb:?}");
        let mut vertical = BezPath::new();
        vertical.move_to(Point::new(0.0, 0.0));
        vertical.line_to(Point::new(0.0, 50.0));
        let vb = calligraphic_band(&vertical, 2.0, 0.1, 0.0).bounding_box();
        assert!((vb.width() - 2.0).abs() < 0.05, "{vb:?}");
        // The round pen (stretch 1) is a plain 2 mm stroke with round ends.
        let rb = calligraphic_band(&horizontal, 2.0, 1.0, 0.0).bounding_box();
        assert!(
            (rb.height() - 2.0).abs() < 0.05 && (rb.width() - 52.0).abs() < 0.05,
            "{rb:?}"
        );
    }

    use super::*;
    use crate::geometry::{Rect, Shape as _};

    fn square(x: f64) -> BezPath {
        Rect::new(x, 0.0, x + 10.0, 10.0).to_path(0.01)
    }

    fn square_side(side: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((side, 0.0));
        p.line_to((side, side));
        p.line_to((0.0, side));
        p.close_path();
        p
    }

    #[test]
    fn inside_and_outside_outlines_cover_one_side_of_the_path() {
        use crate::style::{OutlinePosition, Stroke};
        let sq = square_side(20.0);
        let mut st = Stroke::new(crate::Color::BLACK, 2.0);
        // Centred: a plain stroke, no band.
        assert!(outline_band(&sq, &st, 1.0).is_none());
        st.position = OutlinePosition::Inside;
        let inside = outline_band(&sq, &st, 1.0).expect("band");
        let b = inside.bounding_box();
        assert!(
            b.x0.abs() < 0.01 && (b.x1 - 20.0).abs() < 0.01,
            "inside stays in the path: {b:?}"
        );
        // A 2 mm frame inside a 20 mm square: 400 - 256.
        assert!(
            (inside.area().abs() - 144.0).abs() < 0.5,
            "{}",
            inside.area()
        );
        st.position = OutlinePosition::Outside;
        let outside = outline_band(&sq, &st, 1.0).expect("band");
        let b = outside.bounding_box();
        assert!(
            (b.x0 + 2.0).abs() < 0.01 && (b.x1 - 22.0).abs() < 0.01,
            "outside reaches the full width out: {b:?}"
        );
        // Mitred corners: 576 - 400.
        assert!(
            (outside.area().abs() - 176.0).abs() < 0.5,
            "{}",
            outside.area()
        );
        // The scale applies to the width.
        let scaled = outline_band(&sq, &st, 2.0).expect("band");
        assert!((scaled.bounding_box().x0 + 4.0).abs() < 0.01);
        // An open line keeps a centred outline.
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((30.0, 0.0));
        let open = outline_band(&line, &st, 1.0).expect("band");
        let b = open.bounding_box();
        assert!(
            (b.y0 + 1.0).abs() < 0.01 && (b.y1 - 1.0).abs() < 0.01,
            "{b:?}"
        );
        // Hairlines stay plain.
        let mut hair = Stroke::hairline(crate::Color::BLACK);
        hair.position = OutlinePosition::Inside;
        assert!(outline_band(&sq, &hair, 1.0).is_none());
    }

    #[test]
    fn the_mitre_limit_bevels_sharp_corners() {
        use crate::style::Stroke;
        // A narrow spike: about 11 degrees at the tip.
        let mut spike = BezPath::new();
        spike.move_to((0.0, 0.0));
        spike.line_to((50.0, 5.0));
        spike.line_to((0.0, 10.0));
        let mut st = Stroke::new(crate::Color::BLACK, 1.0);
        st.miter_limit = 5.0;
        let sharp = stroke_area(&spike, &st).bounding_box().x1;
        st.miter_limit = 30.0;
        let cut = stroke_area(&spike, &st).bounding_box().x1;
        assert!(sharp > 54.0, "{sharp}");
        assert!(cut < 51.0, "{cut}");
    }

    /// Whether `p` is on the drawn part of `dashes` (within `tol`).
    fn covered(dashes: &BezPath, p: Point, tol: f64) -> bool {
        use kurbo::ParamCurveNearest as _;
        dashes
            .segments()
            .any(|s| s.nearest(p, 1e-6).distance_sq <= tol * tol)
    }

    #[test]
    fn aligned_dashes_put_corners_and_ends_on_dashes() {
        use kurbo::ParamCurveArclen as _;
        let sq = square_side(20.0);
        // 3 on, 2 off: 20 / 5 = 4 repeats per side exactly.
        let d = aligned_dashes(&sq, &[3.0, 2.0]);
        for corner in [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)] {
            assert!(
                covered(&d, Point::new(corner.0, corner.1), 1e-6),
                "{corner:?}"
            );
        }
        // Every side has 4 dashes, the corner ones shared: 16 pieces.
        let pieces = d
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::MoveTo(_)))
            .count();
        assert_eq!(pieces, 16);
        // Gaps fall between them: 2.5 mm from a corner is off.
        assert!(!covered(&d, Point::new(2.5, 0.0), 0.01));
        // A side that does not hold whole repeats is stretched: 23 mm with
        // a 5 mm pattern takes 5 repeats of 4.6 mm, still on the corners.
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((23.0, 0.0));
        let d = aligned_dashes(&line, &[3.0, 2.0]);
        assert!(covered(&d, Point::new(0.0, 0.0), 1e-6));
        assert!(covered(&d, Point::new(23.0, 0.0), 1e-6));
        let total: f64 = d.segments().map(|s| s.arclen(1e-6)).sum();
        // 5 dashes of 2.76 mm (two halves at the ends): 3/5 of the length.
        assert!((total - 23.0 * 0.6).abs() < 1e-3, "{total}");
        // A smooth closed curve has no corners: one run around it.
        let circle = kurbo::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let d = aligned_dashes(&circle, &[3.0, 2.0]);
        assert!(!d.elements().is_empty());
        // In the outline: aligned dashes go through the band.
        let mut st = crate::style::Stroke::new(crate::Color::BLACK, 1.0);
        st.dash = vec![3.0, 2.0];
        assert!(outline_band(&sq, &st, 1.0).is_none());
        st.align_dashes = true;
        let band = outline_band(&sq, &st, 1.0).expect("band");
        assert!(band.area().abs() > 1.0);
        // Degenerate patterns leave the path as it is.
        assert_eq!(aligned_dashes(&sq, &[0.0, 0.0]).elements(), sq.elements());
        assert_eq!(aligned_dashes(&sq, &[]).elements(), sq.elements());
    }

    #[test]
    fn stroke_area_covers_the_outline_and_hairlines_cover_nothing() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((50.0, 0.0));
        let s = crate::style::Stroke::new(crate::Color::BLACK, 2.0);
        let a = stroke_area(&line, &s);
        let b = a.bounding_box();
        assert!(
            (b.width() - 50.0).abs() < 0.01 && (b.height() - 2.0).abs() < 0.01,
            "{b:?}"
        );
        assert!((a.area().abs() - 100.0).abs() < 0.5, "{}", a.area());
        // Round caps reach half the width past the ends.
        let mut round = s.clone();
        round.cap = crate::style::LineCap::Round;
        assert!((stroke_area(&line, &round).bounding_box().width() - 52.0).abs() < 0.05);
        // Dashes leave gaps: half on, half off.
        let mut dashed = s.clone();
        dashed.dash = vec![2.5, 2.5];
        let d = stroke_area(&line, &dashed).area().abs();
        assert!((d - 50.0).abs() < 2.0, "{d}");
        let hair = crate::style::Stroke::hairline(crate::Color::BLACK);
        assert!(stroke_area(&line, &hair).elements().is_empty());
    }

    #[test]
    fn weld_two_overlapping_squares() {
        let w = overlay(&square(0.0), &square(5.0), Op::Weld);
        let b = w.bounding_box();
        assert!((b.x0 - 0.0).abs() < 1e-6 && (b.x1 - 15.0).abs() < 1e-6);
        assert!((w.area().abs() - 150.0).abs() < 1e-3);
    }

    #[test]
    fn separate_cubic_paths_keep_original_control_points() {
        use crate::geometry::Shape as _;
        let left = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let right = crate::geometry::Circle::new((40.0, 0.0), 10.0).to_path(0.01);
        let weld = overlay(&left, &right, Op::Weld);
        let expected = join_exact_paths(&[left.clone(), right.clone()]);
        assert_eq!(weld.elements(), expected.elements());
        assert_eq!(overlay_parts(&left, &right, Op::Weld).len(), 2);
        assert_eq!(overlay_parts(&left, &right, Op::Exclude).len(), 2);
        assert!(overlay(&left, &right, Op::Intersect).elements().is_empty());
        assert_eq!(overlay(&left, &right, Op::Trim).elements(), left.elements());
        assert!(weld.elements().iter().any(|e| matches!(e, PathEl::CurveTo(_, _, _))));
    }

    #[test]
    fn diagonal_non_crossing_paths_preserve_exact_nodes_despite_overlapping_bounds() {
        let mut lower = BezPath::new();
        lower.move_to((0.0, 0.0));
        lower.line_to((10.0, 0.0));
        lower.line_to((0.0, 10.0));
        lower.close_path();

        let mut upper = BezPath::new();
        upper.move_to((8.0, 8.0));
        upper.line_to((18.0, 8.0));
        upper.line_to((8.0, 18.0));
        upper.close_path();

        let a = lower.bounding_box();
        let b = upper.bounding_box();
        assert!(a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1);
        assert_eq!(
            overlay(&lower, &upper, Op::Weld).elements(),
            join_exact_paths(&[lower.clone(), upper.clone()]).elements()
        );
        assert_eq!(overlay_parts(&lower, &upper, Op::Weld).len(), 2);
        assert!(overlay(&lower, &upper, Op::Intersect).elements().is_empty());
        assert_eq!(overlay(&lower, &upper, Op::Trim).elements(), lower.elements());
    }

    #[test]
    fn chord_clearance_checks_real_distance_not_only_overlapping_boxes() {
        let a = [10.0, 0.0];
        let b = [0.0, 10.0];
        let c = [18.0, 8.0];
        let d = [8.0, 18.0];
        assert!(!chord_bounds_near(a, b, c, d));
        assert!(chord_bounds_near(
            [0.0, 0.0], [10.0, 10.0],
            [0.0, 10.0], [10.0, 0.0]
        ));
        assert!(chord_bounds_near(
            [0.0, 0.0], [10.0, 0.0],
            [10.0, 0.0], [20.0, 0.0]
        ));
    }

    #[test]
    fn nested_circles_preserve_cubics_and_make_editable_holes() {
        use crate::geometry::Shape as _;
        let outer = crate::geometry::Circle::new((0.0, 0.0), 25.0).to_path(0.01);
        let inner = crate::geometry::Circle::new((0.0, 0.0), 8.0).to_path(0.01);
        let trim = overlay(&outer, &inner, Op::Trim);
        let diff_area = outer.area().abs() - inner.area().abs();
        assert!((trim.area().abs() - diff_area).abs() < 1e-3);
        assert!(trim.contains(Point::new(15.0, 0.0)));
        assert!(!trim.contains(Point::new(0.0, 0.0)));
        assert_eq!(trim.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 2);
        assert_eq!(
            trim.elements().iter().filter(|e| matches!(e, PathEl::CurveTo(_, _, _))).count(),
            outer.elements().iter().filter(|e| matches!(e, PathEl::CurveTo(_, _, _))).count()
                + inner.elements().iter().filter(|e| matches!(e, PathEl::CurveTo(_, _, _))).count()
        );
        let parts = overlay_parts(&outer, &inner, Op::Trim);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].elements(), trim.elements());
        assert_eq!(overlay(&outer, &inner, Op::Weld).elements(), outer.elements());
        assert_eq!(overlay(&outer, &inner, Op::Intersect).elements(), inner.elements());
        assert!(overlay(&inner, &outer, Op::Trim).elements().is_empty());
        let reverse_exclude = overlay(&inner, &outer, Op::Exclude);
        assert!((reverse_exclude.area().abs() - diff_area).abs() < 1e-3);
    }

    #[test]
    fn opposite_winding_on_a_nested_cut_still_produces_a_hole() {
        use crate::geometry::Shape as _;
        let outer = crate::geometry::Circle::new((0.0, 0.0), 30.0).to_path(0.01);
        let inner = crate::geometry::Circle::new((0.0, 0.0), 6.0).to_path(0.01);
        let opposite = reverse_closed_contour(&inner).expect("simple closed curve");
        let trim = overlay(&outer, &opposite, Op::Trim);
        assert!(!trim.contains(Point::new(0.0, 0.0)));
        assert!(trim.contains(Point::new(15.0, 0.0)));
        assert!((trim.area().abs() - (outer.area().abs() - inner.area().abs())).abs() < 1e-3);
        assert!(trim.elements().iter().any(|e| matches!(e, PathEl::CurveTo(_, _, _))));
    }

    #[test]
    fn exact_curve_detection_rejects_invalid_control_points() {
        let mut invalid = BezPath::new();
        invalid.move_to((0.0, 0.0));
        invalid.curve_to(
            Point::new(f64::NAN, 0.0),
            Point::new(1.0, 2.0),
            Point::new(2.0, 0.0),
        );
        invalid.line_to((0.0, 1.0));
        invalid.close_path();
        assert!(simple_closed_contour(&invalid).is_none());
    }

    #[test]
    fn identical_curved_shapes_keep_exact_nodes_for_idempotent_operations() {
        use crate::geometry::Shape as _;
        let a = crate::geometry::Circle::new((0.0, 0.0), 12.0).to_path(0.01);
        assert_eq!(overlay(&a, &a, Op::Weld).elements(), a.elements());
        assert_eq!(overlay(&a, &a, Op::Intersect).elements(), a.elements());
        assert!(overlay(&a, &a, Op::Trim).elements().is_empty());
        assert!(overlay(&a, &a, Op::Exclude).elements().is_empty());
        assert_eq!(overlay_parts(&a, &a, Op::Weld)[0].elements(), a.elements());
    }

    #[test]
    fn simplify_of_non_overlapping_curves_preserves_quadratic_and_cubic_nodes() {
        let mut curved = BezPath::new();
        curved.move_to((0.0, 0.0));
        curved.quad_to((10.0, 0.0), (10.0, 10.0));
        curved.curve_to((5.0, 15.0), (0.0, 15.0), (0.0, 10.0));
        curved.close_path();
        assert!(simple_closed_contour(&curved).is_some());
        assert_eq!(simplify(&curved).elements(), curved.elements());
        assert!(curved.elements().iter().any(|el| matches!(el, PathEl::QuadTo(_, _))));
        assert!(curved.elements().iter().any(|el| matches!(el, PathEl::CurveTo(_, _, _))));
    }

    #[test]
    fn simplify_self_crossing_contours_uses_general_overlay() {
        let mut self_crossing = BezPath::new();
        self_crossing.move_to((0.0, 0.0));
        self_crossing.line_to((12.0, 12.0));
        self_crossing.line_to((0.0, 12.0));
        self_crossing.line_to((12.0, 0.0));
        self_crossing.close_path();
        assert!(simple_closed_contour(&self_crossing).is_none());
        // The polygon solver, not the exact-path early return, receives it.
        assert!(!simplify(&self_crossing).elements().is_empty());
    }

    #[test]
    fn two_crossing_cubic_circles_keep_editable_segments() {
        use crate::geometry::Shape as _;
        let a = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let b = crate::geometry::Circle::new((8.0, 0.0), 10.0).to_path(0.01);
        for op in [Op::Weld, Op::Intersect, Op::Trim] {
            let pieces = exact_curve_crossing(&a, &b, op)
                .expect("two transverse cubic intersections");
            assert!(!pieces.is_empty());
            let result = join_exact_paths(&pieces);
            let cubic_count = result.elements().iter()
                .filter(|el| matches!(el, PathEl::CurveTo(_, _, _))).count();
            assert!(cubic_count > 0 && cubic_count < 24, "{op:?}: {cubic_count}");
            assert_eq!(overlay_parts(&a, &b, op).len(), pieces.len());
            let left_only = result.contains(Point::new(-8.0, 0.0));
            let overlap = result.contains(Point::new(4.0, 0.0));
            let right_only = result.contains(Point::new(16.0, 0.0));
            match op {
                Op::Weld => assert!(left_only && overlap && right_only),
                Op::Intersect => assert!(!left_only && overlap && !right_only),
                Op::Trim => assert!(left_only && !overlap && !right_only),
                Op::Exclude => unreachable!(),
            }
        }
    }

    #[test]
    fn perpendicular_ellipses_have_four_parametric_crossings() {
        use crate::geometry::Shape as _;
        let wide = crate::geometry::Ellipse::new(
            (0.0, 0.0), (14.0, 6.0), 0.0
        ).to_path(0.01);
        let tall = crate::geometry::Ellipse::new(
            (0.0, 0.0), (6.0, 14.0), 0.0
        ).to_path(0.01);
        let pieces = exact_curve_crossing(&wide, &tall, Op::Intersect)
            .expect("four transverse cubic intersections");
        let result = join_exact_paths(&pieces);
        assert!(!result.elements().is_empty());
        assert!(result.contains(Point::new(0.0, 0.0)));
        assert!(!result.contains(Point::new(10.0, 0.0)));
        assert!(result.elements().iter()
            .any(|el| matches!(el, PathEl::CurveTo(_, _, _))));
    }

    #[test]
    fn curve_curve_tangent_remains_on_conservative_fallback() {
        use crate::geometry::Shape as _;
        let a = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let b = crate::geometry::Circle::new((20.0, 0.0), 10.0).to_path(0.01);
        for op in [Op::Weld, Op::Intersect, Op::Trim] {
            assert!(exact_curve_crossing(&a, &b, op).is_none());
        }
        assert!(exact_curve_crossing(&a, &b, Op::Exclude).is_none());
    }

    #[test]
    fn derivative_and_bounding_hull_cover_cubic_control_points() {
        let cubic = kurbo::CubicBez::new(
            (0.0, 0.0), (2.0, 15.0), (8.0, -5.0), (10.0, 10.0)
        );
        let segment = kurbo::PathSeg::Cubic(cubic);
        let bounds = control_bounds(&segment);
        assert_eq!(bounds, (0.0, -5.0, 10.0, 15.0));
        let start = curve_velocity(&segment, 0.0);
        let end = curve_velocity(&segment, 1.0);
        assert!((start - (cubic.p1 - cubic.p0) * 3.0).hypot() < 1e-9);
        assert!((end - (cubic.p3 - cubic.p2) * 3.0).hypot() < 1e-9);
    }

    #[test]
    fn curve_crossing_straight_clip_keeps_cubics_in_weld_intersect_and_trim() {
        use crate::geometry::Shape as _;
        let circle = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let clip = Rect::new(-5.0, -15.0, 3.0, 15.0).to_path(0.01);
        let originals = circle.elements()
            .iter()
            .filter(|el| matches!(el, PathEl::CurveTo(_, _, _)))
            .count();
        assert!(originals > 0);
        for op in [Op::Weld, Op::Intersect, Op::Trim] {
            let pieces = exact_curve_crossing(&circle, &clip, op)
                .expect("four clean crossings should keep cubic segments");
            let result = join_exact_paths(&pieces);
            let cubics = result.elements()
                .iter()
                .filter(|el| matches!(el, PathEl::CurveTo(_, _, _)))
                .count();
            assert!(cubics > 0 && cubics <= originals + 8, "{op:?}: {cubics}");
            assert_eq!(overlay(&circle, &clip, op).elements(), result.elements());
            assert!(!pieces.is_empty());
            match op {
                Op::Weld => {
                    assert!(result.contains(Point::new(9.0, 0.0)));
                    assert!(result.contains(Point::new(0.0, 12.0)));
                }
                Op::Intersect => {
                    assert!(result.contains(Point::new(0.0, 0.0)));
                    assert!(!result.contains(Point::new(9.0, 0.0)));
                    assert!(!result.contains(Point::new(0.0, 12.0)));
                }
                Op::Trim => {
                    assert!(result.contains(Point::new(9.0, 0.0)));
                    assert!(!result.contains(Point::new(0.0, 0.0)));
                }
                Op::Exclude => unreachable!(),
            }
        }
    }

    #[test]
    fn straight_subject_and_curved_clip_retain_curve_segments() {
        use crate::geometry::Shape as _;
        let circle = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let subject = Rect::new(-5.0, -15.0, 3.0, 15.0).to_path(0.01);
        for op in [Op::Weld, Op::Intersect, Op::Trim] {
            let parts = exact_curve_crossing(&subject, &circle, op)
                .expect("clean straight-curve intersections");
            let result = join_exact_paths(&parts);
            assert!(result.elements().iter().any(|el|
                matches!(el, PathEl::CurveTo(_, _, _))));
            assert_eq!(overlay_parts(&subject, &circle, op).len(), parts.len());
        }
    }

    #[test]
    fn touching_curve_and_straight_boundary_use_polygon_fallback() {
        use crate::geometry::Shape as _;
        let circle = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let touching = Rect::new(10.0, -15.0, 20.0, 15.0).to_path(0.01);
        assert!(exact_curve_crossing(&circle, &touching, Op::Weld).is_none());
        assert!(exact_curve_crossing(&circle, &touching, Op::Intersect).is_none());
        assert!(exact_curve_crossing(&circle, &touching, Op::Trim).is_none());
        assert!(exact_curve_crossing(&circle, &touching, Op::Exclude).is_none());
    }

    #[test]
    fn touching_and_crossing_paths_use_the_general_solver() {
        use crate::geometry::Shape as _;
        let a = crate::geometry::Circle::new((0.0, 0.0), 10.0).to_path(0.01);
        let crossing = crate::geometry::Circle::new((10.0, 0.0), 10.0).to_path(0.01);
        let touching = crate::geometry::Circle::new((20.0, 0.0), 10.0).to_path(0.01);
        assert!(exact_non_crossing_parts(&a, &crossing, Op::Weld).is_none());
        assert!(exact_non_crossing_parts(&a, &touching, Op::Weld).is_none());
        assert!(!overlay(&a, &crossing, Op::Intersect).elements().is_empty());
    }

    #[test]
    fn offset_grows_and_shrinks() {
        let grown = offset(&square(0.0), 2.0);
        let b = grown.bounding_box();
        assert!(
            (b.x0 + 2.0).abs() < 0.05 && (b.x1 - 12.0).abs() < 0.05,
            "{b:?}"
        );
        let shrunk = offset(&square(0.0), -2.0);
        let b = shrunk.bounding_box();
        assert!(
            (b.x0 - 2.0).abs() < 0.05 && (b.x1 - 8.0).abs() < 0.05,
            "{b:?}"
        );
    }

    #[test]
    fn offset_corners_are_mitred_round_or_bevelled() {
        let area = |j: Join| offset_with(&square(0.0), 2.0, j).area().abs();
        assert!(
            (area(Join::Miter) - 196.0).abs() < 0.2,
            "{}",
            area(Join::Miter)
        );
        let round = 180.0 + std::f64::consts::PI * 4.0;
        assert!(
            (area(Join::Round) - round).abs() < 0.3,
            "{}",
            area(Join::Round)
        );
        assert!(
            (area(Join::Bevel) - 188.0).abs() < 0.2,
            "{}",
            area(Join::Bevel)
        );
        assert!(offset_with(&square(0.0), f64::NAN, Join::Miter)
            .elements()
            .is_empty());
    }

    #[test]
    fn intersect_and_trim() {
        let i = overlay(&square(0.0), &square(5.0), Op::Intersect);
        assert!((i.area().abs() - 50.0).abs() < 1e-3);
        let t = overlay(&square(0.0), &square(5.0), Op::Trim);
        assert!((t.area().abs() - 50.0).abs() < 1e-3);
        assert!((t.bounding_box().x1 - 5.0).abs() < 1e-6);
    }
}
