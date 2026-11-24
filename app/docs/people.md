# People

People finds the faces in your photos, groups the ones that look alike, and lets you name
them. It is **off until you turn it on**. Face data is biometric: Immage keeps it on this
computer, in a local database, never uploads it, and you can delete all of it in one click.
Nothing is written to your photos or their sidecars.

Open it with the **People** tab in the top strip, **View > People**, or **Shift+P** (press
it again to go back to the grid).

## Turning it on

Until it is on, the People view is one card with three steps.

1. **Model.** The Faces model (YuNet and SFace, about 39 MB). The step shows its state and
   the same Download button as Preferences > General > AI models. A model that is already
   installed there is already done here.
2. **What to scan.** The whole Catalog, or only the open folder (then nothing else is scanned).
3. **Start.** Tick *I understand this stores face data on this computer*, then **Turn on
   People**. The box is your consent and is recorded on this computer. The button says why
   it waits (no model, box not ticked) in the line above it.

*Use eye detection only* is listed for the closed-eye detection planned for Auto Cull and is
not available yet.

## The view

A strip at the top shows the scan: *Finding faces: 1,204 of 48,210 photos - about 2 h left*,
with **Pause** (and **Resume**). The same line is in the activity area under the photo, where
**Stop** pauses it. What has been found is kept. A problem shows in the strip with **Try again**. Quiet notes sit beside it: *N photos could
not be read - Show* (a list of the photos, with **Try again**), *N folders offline* (their
faces are kept and scanning resumes when the drive is back) and *N network folders not
scanned*, because network folders are left out of the scan.
The strip, the toolbar, the selection bar and the Undo line at the bottom always have their
place, so nothing jumps when they fill.

On the left, four sections with counts:

| Section | What it holds |
|---|---|
| **People** | the people you named, most photos first |
| **To review** | groups of faces that look alike ("Looks like one person", shown as a 2 x 2 collage of up to four of its faces), and the faces suggested for a person you named ("Maybe Maya", with a dashed outline until you confirm) |
| **Unassigned** | single faces with no group |
| **Ignored** | people you ignored and faces you marked "not a face", dimmed; **Restore** brings them back |

The first time faces appear, the view opens on To review when there is something there. The
tab shows a dot while faces need review. New items are added at the end of a grid while you
look at it, and the view does not change under your pointer: updates wait until the pointer
leaves the grid. The size slider sets the face size, and *Find a person* filters the names.

The right side shows details and the actions for what is selected, plus *Pause*, *Turn off
People...* and *Delete all face data...*. It is hidden when the view is narrower than about
1,100 px; the selection bar above the grid has the main actions then.

## Selecting

As in the grid: click selects one, **Cmd/Ctrl-click** adds or removes, **Shift-click** selects
a run, drag on empty space for a rectangle (it scrolls at the edges), **Cmd/Ctrl+A** selects
all, **Esc** clears. The arrow keys, Home, End and Page Up and Down move; hold **Shift** to
extend. Selection is kept while you scroll.

## Naming

Select a group (or a face, or several) and press **N**, or just **start typing a name**, or
use **Name...**. A box opens under the tile: type to filter the people you already named,
**Up** and **Down** choose, **Enter** names every selected face (they become confirmed) and
**Esc** closes the box with nothing changed. **Tab** names and goes straight on to the next
group. A name that exists is offered, never duplicated; *Create "..." as a new person* is
the last row. Without touching the pointer: **Shift+P**, **Tab** to the grid, arrows to a
group, type *Dad*, **Enter**.

**Enter** (or a double-click) on a group opens its faces, so you can name only some of them;
**Back** returns.

In **To review**, a "Maybe ..." tile has **Confirm** and **Not them**; a face or group can be
marked **Not a face** (the key is Backspace). In **People**: **Rename** (F2 or the button; a
name that exists offers to merge), **Merge...** (two or more people, with a dialog showing
who keeps the name; the default is the person with more photos), **Cover...** (pick the
picture shown for the person), **Ignore** (key I; Restore in Ignored).

Every change shows **Undo** for five seconds at the bottom ("Named 12 faces Dad."). Further
changes of the same kind join the line; Undo takes all of them back. The line waits while the
pointer or the keyboard is on it.

## Turning off and deleting

*Turn off People...* stops scanning and keeps your data. *Delete all face data...* removes
every person, face, group and crop and turns People off. Your photos are not changed. It
cannot be undone.

## Not yet

A person's own page, finding photos by person, closed-eye detection and Preferences > Faces
are planned.
