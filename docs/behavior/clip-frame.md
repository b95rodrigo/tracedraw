# Clip frames

Object > Clip to Frame: objects placed inside a container that clips
them.

## Model

`ShapeKind::ClipFrame { frame, contents }`: the frame is any closed
object, kept whole with its own transform, fill and outline; the
contents keep their own page-space transforms. The frame's fill and
outline are drawn, the contents are clipped to the frame's path.

A frame may be a compound vector path with several closed contours. Its
contents use even-odd clipping: an inner contour forms a hole even when
its drawing direction matches the outer contour. The normal SVG exporter
writes `clip-rule="evenodd"` inside the SVG clip path; PDF uses the
equivalent `W*` clipping operator. The frame's cubic curves stay vector.

Opacity masks are a separate mode and do not use the normal hard-clip
export path. They retain the format-specific behavior described below.

## Native CDR clipping (experimental)

The writer now builds a `LIST clpt` with separately numbered groups
and an `8005` reference on a supported frame's vector object.
Supported frames are closed, visible rectangles, ellipses, polygons
or Bézier paths without live frame effects. Nested contents remain
ordinary editable objects. A single cropped bitmap continues using
the established bitmap crop encoding.

The available public-format and fixture evidence confirms the
`clpt` layout for version 12 only. For modern CDR versions, the
writer uses the same records provisionally; independent applications
have not confirmed round-trip support. Unsupported/open frames still
degrade to an unclipped group. The SVG and PDF exporters have their
own hard-clip handling independent of this CDR support.

Tests for multi-container references, Bézier frames, object names and
editable contents have been committed but not executed.

## Commands

- Place Inside Frame: with contents selected, click the frame; the
  contents keep their position.
- Extract Contents: the contents become ordinary objects again; the frame
  stays (an empty frame shows a hatched preview).
- Edit Contents: enters the clip frame; only the contents are editable,
  the rest of the page is dimmed and the frame is drawn as an outline.
  Finish Editing Contents leaves it. While editing, the property bar
  shows Finish.
- Lock Contents to Frame (default on, stored as `clip_frame.unlocked` in
  the object data when off): with the lock on, moving, scaling or
  rotating the frame transforms the contents too. With it off, the frame
  moves alone and the contents stay where they are
  (`compensate_unlocked_clip_frames` undoes the transform on them).

## Picking

A click selects the clip frame (anywhere in its box). Ctrl+click
selects the object of the contents under the pointer, only inside the
frame where the contents show; further Ctrl+clicks go into a group
among the contents. Ctrl+Alt+click digs through the contents and then
the clip frame (see `pick-tool.md`).

## Opacity mask

Object > Clip to Frame > Opacity Mask (a check item) turns the selected clip
frames into opacity masks and back: the frame is no longer drawn and no
longer clips hard; its brightness (its fill, gradients and patterns
included, through the luminance of the colour) sets how much of the
contents shows, white fully, black not at all, nothing outside the
frame. Invert Mask swaps that (black shows). Place inside, extract, edit
in place and lock work as for any clip frame. One undo step each.

- Screen and bitmaps: the frame is drawn alone and its luminance masks
  the contents' layer (`Shape::opacity_mask`, `crates/render`).
- SVG: a `<mask>` holding the frame's fill (inverted through a
  `feColorMatrix` filter), on a group of the contents.
- PDF: the masked object goes in as a picture with its transparency
  (300 dpi, at most 40 million pixels); PDF/X-1a flattens it with what
  lies under it.

## Frame Type (object context menu)

- Create Empty Clip Frame: each selected object that is not a clip
  frame becomes one holding nothing, in place (turned or not), as Place
  Inside Frame with no contents does; one undo step. Pale when nothing
  selected can become one.
- No Frame: each selected empty clip frame becomes the object it was
  made from, with its id; one undo step. Pale without one.
- Create Empty Text Frame (one object selected): the object gives way to
  an empty paragraph text frame of its size (its bounds) in its place,
  in the font, size, style, alignment and hyphenation new text takes;
  one undo step. The frame stays selected (and stays when left empty)
  for the Text tool.

## Checks

- Given a locked clip frame, when the frame is moved by (10, 0), then the
  contents move by (10, 0).
- Given an unlocked clip frame, when the frame is moved by (10, 0), then
  the contents' page bounds are unchanged.
- Given a `.cdr` bitmap with a crop outline smaller than the image, then
  the reader produces a clip frame of the bitmap inside the crop path
  (confirmed with the 2019 banner file).
- `render::tests::opacity_masks_show_the_contents_by_the_frame_brightness`,
  `svg::tests::opacity_masks_are_svg_masks`,
  `pdf::tests::opacity_masks_go_in_as_transparent_pictures`,
  `command::tests::only_clip_frames_take_an_opacity_mask`,
  `app::tests::clip_frames_turn_into_opacity_masks_and_back`.
- `frame_type::tests::empty_clip_frames_come_and_go_in_place`,
  `frame_type::tests::an_object_gives_way_to_an_empty_text_frame_of_its_size`.

- `svg::tests::compound_clip_frame_exports_evenodd_and_keeps_cubic_paths`.
- `pdf::text_tests::pdf_compound_clip_frame_uses_evenodd_and_retains_bezier_commands`.
- `pdf::text_tests::boolean_curved_hole_remains_vector_in_svg_and_pdf`.

The last three tests have been committed but not executed. Independently
reopen and inspect exported artwork before claiming output fidelity.
