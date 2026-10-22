# Saving files

File > Save (Ctrl+S) writes the drawing back to its file. A drawing that
has none (new, or opened from a format TraceDraw only reads or converts,
`.cdr` included), and File > Save As (Ctrl+Shift+S), open the Save
Drawing dialog.

## Save Drawing

One window, 760 px wide, the same on every system:

- **Folder** (desktop): an Up button, the path bar (a folder typed there
  opens with Enter; one that does not exist says so) and New folder
  (makes "New folder", "New folder (2)" ... and opens it). Down the left,
  shortcuts to the folders that exist: Home, Desktop, Documents,
  Downloads and Pictures (on Linux the names in
  `~/.config/user-dirs.dirs`), then the drives (Windows) or `/`. On the
  right, 250 px high and scrolling, the folder's contents: folders, then
  the files of the chosen type, by name ignoring case (hidden ones left
  out), with Name, Date modified (local time), Type ("File folder",
  "CDR file") and Size (KB, MB from 1 MB). A click on a file takes its
  name; a double click on a folder opens it.
- **File name**: the drawing's file name, else its title, else
  "Graphic1", with the type's extension; a name typed with either
  extension, or none, gets the chosen type's.
- **Save as type**: TraceDraw (`*.tdraw`, the native JSON format, which
  keeps everything) or CDR files (`*.cdr`). It starts at `.cdr` for a
  drawing that came from or was saved to a `.cdr` file.
- **Version**: for TraceDraw files the `.tdraw` format version, shown
  as "v0" with nothing to choose while it is the only one
  (`docs/tdraw-format.md`); for `.cdr` 27 down to 14, each with its
  release name ("v14 (X4)", "v27 (2026)").
- **Embed colour profile (size)** with "Colour profile: name" (`.cdr`
  only): on, the drawing's CMYK profile is stored in the file; off, it
  is left out. Only version 16 and later store one, so the box is off
  and greyed for older versions and for a drawing without a profile
  ("Colour profile: none"). **Embed fonts (size)** sits beside it: on,
  the faces the text uses go into the file when their licence allows
  it; every face is listed in the file's font table either way. It is
  greyed below version 16 and when no face may be embedded. Both
  choices stay for the next Save As and for Save.
- Only the versions checked in other programs are offered (14 to 27).
- **Title**, **Subject** and **Rating** (five stars; clicking the lit
  last star takes it off): the document's properties, also in File >
  Document Properties; changed ones are set (one undo step each) before
  the file is written.
- **Save** (or Enter) writes the file; a file that exists asks "<name>
  already exists. Replace it?" with Replace and Cancel. Cancel closes
  without writing. The dialog starts in the drawing's folder, else the
  last folder saved to, else Documents, else home.
- In the browser, where every save is a download, the dialog keeps the
  file name, type, version and properties, and says the browser saves
  the file in its downloads folder.

## CDR preservation limitations

The ZIP package metadata includes names from nested groups and clipping
frames, including text inside those objects. Versions 14 and 15 also
index the font families used by nested text. This metadata is an index,
not a guarantee that every live object is faithfully encoded in the
native document stream.

The writer now emits native `clpt` references for eligible closed
vector clipping frames in version 12 and later, retaining frame and
contents as editable objects. The record layout is supported by public
documentation and one version 12 fixture only. Its use in versions
13 to 27 is **experimental** and prompts a fidelity notice, even though
the project's own reader can reconstruct this arrangement.

Unsupported frames, open contours and versions before 12 continue
to use an unclipped group approximation. A second narrow native effect
writer now supports an **invert lens** on one opaque vector object.
The tagged binary layout is evidenced in version 15 only; its use
in other versions is experimental and emits a compatibility notice.
Other lenses, stacked effects and advanced text features can still
differ after reopening. For transparent gradients, only a single
normal-blend, two-stop mask with supported options is written as a
native editable transparency in version 16+. More complicated masks
are approximated by uniform opacity and emit an explicit fidelity
notice, rather than losing intermediate stops silently. Prefer
`.tdraw` to retain a fully editable original while interoperability
is being improved.

For known lossy constructs, saving a CDR file now appends a "CDR
warning" to the status line and writes the details to the application
log. This does not block the save. It warns about unsupported
clipping frames, experimental native vector clipping in versions other
than 12, hidden objects omitted by the target version, expanded live
effects, paragraph text frame conversion and opacity mask limitations.
A source-supported single native invert lens no longer reports static
expansion, but versions other than 15 show an unverified-layout notice.
Complex gradient masks (extra stops, special blending, repeats
or stacked effects) also trigger a separate warning when converted
to uniform opacity. These source-based warnings are **not exhaustive**;
absence of a warning does not mean the file has been independently verified.

Source regression tests for nested metadata, style-string escaping,
compound editable Bézier paths and loss notices are committed but
unexecuted. Opening the written file in an independent program remains
a release gate.

## `.cdr` versions

A program opens `.cdr` files of its own version and older ones, so the
version decides who can open a file:

- Save As starts at the newest version, 27, whatever the drawing came
  from.
- The version chosen stays with the drawing: Save writes the same
  version again without asking. Each open drawing keeps its own.
- TraceDraw saves only the versions checked in other programs, 14 to 27.
  A drawing opened from an older `.cdr` (versions 1 to 13) keeps its
  file, but Save does not write that version: the status line says
  which version Save writes ("Save writes version 14."), and Save opens
  "Older file version": the drawing came from version N, TraceDraw does
  not save in it, choose a version from 14 to 27; Choose Version...
  (or Enter) opens Save As on version 14, Cancel saves nothing. Once
  saved in a checked version, Save writes it without asking. A file
  newer than 27 is saved as 27. Options > Global > File Formats (version
  27 at first) is used for a drawing saved to a `.cdr` file without a
  version of its own.
- `tracedraw-cli convert in.svg out.cdr` writes version 27, or the
  version in `TRACEDRAW_CDR_VERSION` (14 to 27; another version is
  refused with exit code 2). Versions 7 to 13 are experimental and are
  written only with `TRACEDRAW_CDR_EXPERIMENTAL=1`, for testing them in
  other programs. The MCP `save` tool takes an optional `version`, 14 to
  27 (`mcp.md`).

What each version's file holds is in `docs/cdr-format.md`, Writing.

## Checks

- Given a folder with a subfolder, a `.cdr` and a `.txt` file, the
  TraceDraw type lists the subfolder only and CDR the subfolder and the
  `.cdr` file; saving "poster" as CDR version 15 with a title, subject
  and rating writes `poster.cdr` there (version 15), sets the properties
  and remembers the folder; the next Save As starts there as `.cdr`
  version 27; an existing name asks first and Replace overwrites it; a
  TraceDraw file is written with its own extension, in the format version
  chosen (0), which Ctrl+S keeps and opening the file brings back
  (`ui::save_as::tests::save_as_chooses_folder_name_type_version_and_properties`).
- An empty name, a bare extension and a missing folder save nothing and
  say why; typed extensions become the type's
  (`ui::save_as::tests::names_and_folders_are_checked`).
- A version 21 file opened: Ctrl+S opens Save As (an opened `.cdr` is
  never overwritten in place), and once saved Ctrl+S writes it again; a
  version 9 file opened says Save writes version 14, Ctrl+S writes
  nothing and asks for a version, Save As opens on 14 and saves version
  14, after which Ctrl+S saves without asking; versions 3 and 99 are
  saved as 14 and 27
  (`ui::save_as::tests::a_drawing_opened_from_a_cdr_file_keeps_its_version`).
- A drawing with a CMYK profile offers it by name and size; saved with
  the box on, the version 27 file holds the profile, with the box off it
  does not, and the next Save As keeps the box off
  (`ui::save_as::tests::the_cmyk_profile_goes_along_unless_left_out`).
- The dialog draws, folders open (`ui::save_as::tests::the_dialog_draws_and_opens_folders`),
  and the Linux folder names are read from the user's settings
  (`ui::save_as::tests::linux_folder_names_come_from_the_users_settings`).
- The MCP `save` tool writes version 21 when asked and refuses versions 3
  and 23 (`mcp::tests::export_and_save_write_files`).
- Every version from 7 to 22 reads back with its pages, layers, objects,
  text runs and alignment (`write::tests` in `crates/cdr`).
- Checked on screen: the dialog with its shortcuts, list, type list,
  version and stars; Save as CDR wrote the file in the folder shown.
