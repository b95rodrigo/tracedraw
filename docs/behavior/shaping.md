# Shaping

Object > Shaping, the property bar's shaping buttons (with two or more
objects selected) and the Shaping panel.

## Operations

The target gives the result its fill, outline and opacity; the other
selected objects are the sources. With objects picked one by one
(click, Shift+click) the target is the one selected last; with a
selection made by a marquee or a freehand region, the target is the
bottom-most object of it (as long as the selection is not changed by a
click afterwards). Checked by
`ops::curves_and_target_tests::shaping_targets_the_last_clicked_object_but_the_bottom_one_of_a_marquee`.

| Operation | Needs | Result | What stays (menu, property bar) |
|---|---|---|---|
| Weld | 2 | one object covering all of them | nothing |
| Trim | 2 | the target without what the sources cover | the sources |
| Intersect | 2 | the area all of them share | the sources and the target |
| Simplify | 1 | each object without what objects above it cover | the objects, changed in place |
| Front Minus Back | 2 | the front object without the ones behind (its attributes) | nothing |
| Back Minus Front | 2 | the back object without the ones in front (its attributes) | nothing |
| Boundary | 1 | an outline around the selection (the target's attributes) | every object |

The result goes on top of the active layer and becomes the selection; an
empty result changes nothing and the status bar says so. Each operation
is one undo step.

### Editable curve preservation

Weld, Trim, Intersect and Exclude now have a conservative exact-path case:
two finite, non-self-intersecting, single closed contours whose real
boundaries stay clearly separated (including cases with overlapping axis-
aligned bounds), or that are strictly nested, keep their lines and cubic/quad
control points. A subtraction that makes a hole keeps both original
contours, reversing the inner one if needed for non-zero fill.

When both selected inputs have identical simple closed paths, Weld and
Intersect keep the exact original contour, while Trim and Exclude produce
the empty result. Simplify also preserves any single simple closed contour
without changing its node types or control points. This avoids destructive
flattening where there is no overlap to resolve.

The experimental parametric crossing solver handles Weld, Intersect and
Trim for two simple closed contours made from straight, quadratic and
cubic segments. Straight/curve pairs use a direct line intersection.
Curve/curve pairs isolate potential intersections with recursively split
Bezier control hulls and refine roots with Newton iterations.

When all crossings and resulting paths are unambiguous, each output edge
is an original, parameter-split curve segment. The operation does not
refit polygons into new curves. A bounded work budget, tangent checks,
closed-loop topology checks and a fill-membership sanity check reject
uncertain results.

The fallback still applies to coincident edges, tangencies, junctions at
existing nodes, overlapping/degenerate geometry, Exclude on crossing
boundaries, self-intersections, open paths and multi-contour shapes.
Ambiguous inputs use flattened polygons at 0.02 mm tolerance, so this is
not yet a universally exact curve boolean implementation. This solver
was source-integrated without runtime compilation or tests.

Regression tests in `crates/core/src/shaping.rs` cover disjoint curves,
including overlapping bounds, nested curve holes, exact control-point
retention, winding direction, invalid input rejection and deliberate
fallback on touching/crossing paths. Additional unexecuted tests cover
straight/curved crossing contours in both operand orders, two crossing
cubic circles, four intersections between perpendicular ellipses,
parametric derivative/control bounds, and tangent fallback. These tests
have been added but have not been executed.

## Panel

- Seven picture buttons choose the operation; under them its name, a
  large picture and what it does, and that the last object selected is
  the target.
- Weld, Trim and Intersect each keep their own choice of what to leave:
  "Leave original source object" and "Leave original target object"
  (Weld: neither, Trim: the sources, Intersect: both, as the menus do).
  The choices and the operation are kept in the preferences
  (`panels.shaping_op`, `panels.shaping_leave`).
- The button under them, named after the operation, applies it; it is
  pale until enough objects are selected, and a line says how many.

## Checks

- `ui::shaping_docker::tests::what_the_operations_leave_of_the_originals`
- `ui::shaping_docker::tests::the_docker_draws_every_operation`
- `ui::propbar::tests::shaping_and_grouping_show_for_several_objects_groups_and_parts`
