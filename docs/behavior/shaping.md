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
two finite, non-self-intersecting, single closed contours that are clearly
separated or strictly nested keep their existing lines and cubic/quad
control points. A subtraction that makes a hole keeps both original
contours, reversing the inner one if needed for non-zero fill.

Touching or crossing boundaries, complex or open paths and multiple
subpaths still go through the polygon overlay engine. That engine flattens
curves at 0.02 mm tolerance; it does not yet preserve editable original
segments at intersections. Do not treat this partial improvement as a
complete curve-preserving boolean implementation.

Regression tests in `crates/core/src/shaping.rs` cover disjoint cubic
curves, nested curve holes, exact control-point retention and deliberate
fallback on touching/crossing paths. Their execution needs a local Rust
toolchain when hosted CI is not available.

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
