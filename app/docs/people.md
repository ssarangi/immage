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
2. **What to scan.** Only the open folder (the default while a folder is open, and then nothing else is scanned), or the whole Catalog (the default when no folder is open; it may include network sources, which are left out).
3. **Start.** Tick *I understand this stores face data on this computer*, then **Turn on
   People**. The box is your consent and is recorded on this computer. The button says why
   it waits (no model, box not ticked) in the line above it.

*Use eye detection only* is listed for closed-eye detection for culling and is not available yet. Today
Auto Cull and Scenes use the faces People already found, plus closed-eye detection when you have supplied the eye
model file (it is never downloaded); see [Auto cull](library.md#starting-a-cull).

## The view

A strip at the top shows the scan: *Finding faces: 1,204 of 48,210 photos - about 2 h left* (the time appears once photos have been timed; for a folder scan the total is the folder's photo count),
with **Pause** (and **Resume**). The same line is in the activity area under the photo, where
**Stop** pauses it. What has been found is kept. A problem shows in the strip with **Try again**. Quiet notes sit beside it: *N photos could
not be read - Show* (a list of the photos, with **Try again**), *N folders offline* (their
faces are kept and scanning resumes when the drive is back) and *N network folders not
scanned*, because network folders are left out of the scan.
The strip, the toolbar, the selection bar and the Undo line at the bottom always have their
place, so nothing jumps when they fill.

The chip at the left of the toolbar says what the scan covers: *Catalog* or *Folder: name*. A group tile
reads *Unnamed group* (hover for "Looks like one person"). On the left, four sections with counts:

| Section | What it holds |
|---|---|
| **People** | the people you named, most photos first |
| **To review** | groups of faces that look alike ("Looks like one person", shown as a 2 x 2 collage of up to four of its faces), and the faces suggested for a person you named ("Maybe Maya", with a dashed outline until you confirm) |
| **Unassigned** | single faces with no group |
| **Ignored** | people you ignored and faces you marked "not a face", dimmed; **Restore** brings them back |

At the top of To review, a **Suggestions** row offers the strongest pair the analysis thinks is one
person (nothing is ever done for you): *These groups may be the same person* (**Merge** joins
them at once, no name needed, and **Not the same** is remembered), *This group looks like Maya*
(**Add to Maya** names its faces Maya, **Not them** is remembered and the pair is not offered
again) or *Maya and Sam may be the same person* (**Merge** opens the merge dialog, **Not the same** is remembered). **Show N more**
lists the others. The row is one fixed line: focus it with Tab and press **Enter** to take the
first action or **Esc** to dismiss the suggestion for this session. A group that looks like someone
named wears a dashed chip with that name on its tile. In To review you can also select two or more groups and press **Merge groups**: the biggest takes the others in. Naming or confirming faces makes the
analysis suggest the alike faces and groups again within about a second.

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

Every change shows **Undo** for five seconds (eight when it touched more than 20 faces) at the bottom ("Named 12 faces Dad."). Further
changes of the same kind join the line; Undo takes all of them back. The line waits while the
pointer or the keyboard is on it. **Ctrl/Cmd+Z** in People takes back the latest change even after the line is gone
(outside a text field; the edit history keeps Ctrl/Cmd+Z everywhere else).

## Faces on the photo

With People on, **Shift+F** (or **View > Show faces on photo**, or the switch in the Library column's **People** card)
draws a box around each face on the open photo in the editor. The boxes follow the picture as you zoom and pan: a solid
outline is a named face, dashed a suggestion, dotted a face nobody has a name for. The name shows on hover or keyboard
focus (Tab moves from face to face, left to right). Click a face to name it or give it to someone else in the name box,
or say **Not Maya** or **Not a face**; each is undoable for a few seconds like the rest of People. The same boxes (and the same Shift+F) work in the Grid's photo preview (F), where they follow Fit and 100%. The boxes are never
part of the photo or an export, and nothing is drawn while People is off.

The **People** card in the Library column (shut until you open it, and absent while People is off) lists the open
photo's faces as chips with a small crop and the name or "Unnamed face". Clicking a chip selects that face's box and
turns the boxes on.

## A person's page

Press **Enter** or double-click a person to open their page: a fixed head with the cover, the name (double-click to
rename), what is known about them, and **Open in Grid**, **Rename**, **Cover...**, **Merge...** and **Ignore**.
The **Photos** tab is the app's own grid of their photos (a thumbnail comes from the disk cache; a click opens the
photo in the editor). **Open in Grid** returns to the Grid with Find's People filter set to this person, shown as
a chip you can remove. The **Faces** tab lists their faces, the suggested ones dashed: select faces and **Confirm**
a suggestion, say **Not Maya** (remembered), **Not a face**, or **Use as cover**. **Esc** or the breadcrumb goes
back to People.

## Finding photos by person

When People is on, Find has a **People** row (in the Photo group; hidden while People is off). Type to add
people: each becomes a chip, and the chip's button flips it between *include* and *leave out*.
**Any of them** keeps photos with at least one of the people, **All together** only photos with all of
them. **No people** keeps photos without faces, **Unnamed faces** photos with a face still to name. The
row counts as one filter, is cleared by Clear all, and is kept in saved filters.

Preferences > Faces ([Preferences](preferences.md#faces)) has the same switch and deletion, how strictly faces are grouped
as one person, how thoroughly photos are scanned, Reprocess all faces, the model, and the numbers the face store holds.

## Turning off and deleting

*Turn off People...* stops scanning and keeps your data. *Delete all face data...* removes
every person, face, group and crop and turns People off. Your photos are not changed. It
cannot be undone.

## Not yet

Closed-eye detection and Preferences > Faces are planned.
