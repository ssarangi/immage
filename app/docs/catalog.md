# Catalog

The **Catalog** keeps what Immage reads from a folder once, so the folder's
photos can be counted, searched and browsed later, even when the drive or NAS
they are on is away. It sits beside your **Folders**, not in place of them:
the Folders card works exactly as before, and you can use either, or both.

Immage only reads. Indexing reads file headers and makes thumbnails on your
computer. It never changes, moves or deletes your photos or their sidecars.

## Folders and the Catalog

| | Folders | Catalog |
|---|---|---|
| What it is | A folder you opened | A folder Immage has indexed (a **source**) |
| Needs the drive | Always | Only to index and to open originals |
| Shows | Photos as they are on disk | Photos Immage has read: counts, cached details, thumbnails |
| Where | **Library** panel, **Folders** card | **Library** panel, **Catalog** card |

A folder can be in both lists. Adding a folder to the Catalog does not take it
off the Folders list.

## Adding a folder

- Click **+** in the Catalog card, or **+ Add folder to Catalog…**, and choose
  a folder in Immage's [folder browser](library.md#the-folder-browser).
- Or right-click a folder in **Folders** and choose **Add to Catalog…**.

A dialog says what the folder is (a network share with its host and share, a
drive, or a folder on this computer) and repeats that Immage only reads. Click
**Add and index**. A folder that is already inside a source, or that contains
one, is refused with the reason. A share that is asleep is woken by adding it.
A folder whose drive is not connected cannot be added yet.

## The index

The first index of a source runs in three steps, shown in the Catalog card under
the list:

1. **Finding photos**: listing the folders. The count rises until the whole
   folder has been seen.
2. **Reading metadata**: camera, lens, exposure, time, size, location, rating,
   labels and keywords from each file's header and sidecar.
3. **Making thumbnails**: small previews, made on this computer.

Each step shows its numbers in place (for example "41,860 of 98,200 - 118 a
second"). The time left is a range ("About 9 min left (could be 6 min to 12
min)"), shown only once the speed is steady and never while paused. You can
keep using Immage while it runs: the card says which photos are already usable.

- **Pause** stops after the work in flight and keeps everything done. **Resume**
  continues. **Stop** ends the index for good; a later **Check for changes**
  starts again from the folder.
- If you quit Immage, an unfinished index shows as paused and **Resume**
  continues it.
- When it ends, the card says "is ready" with the number of photos, how many
  could not be read (they stay listed, nothing is hidden or removed) and
  **Open**.

If something goes wrong the card says so in plain words and nothing is marked
missing: the connection was lost (it waits for the share), the disk is full (so
thumbnails stopped), some folders did not answer in time, or the index stopped
with a reason. **Try again** starts it again.

## The Catalog card

Each source is a row with its photo count and one symbol. The symbol is also a
word in its tooltip, and the shape is different for every state:

| Symbol | State |
|---|---|
| Solid dot | Online |
| Ring that fills | Indexing (its fraction shows in the tooltip) |
| Two bars | Paused |
| Triangle | Needs attention (the tooltip says why) |
| Empty ring | Offline |
| Dotted ring | A share that is asleep, or not indexed yet |

Sources on the same drive or share are grouped under its name, with a word for
whether it is online. Each source is the top of its own folder tree, drawn
exactly like the [Folders](library.md#folders) tree: click the arrow to open it
into its folders, and click a folder to open it in the grid. Right-click a
source, or use the **…** button, for **Open in Library**, **Cull this
folder…**, **Statistics for this folder**, **Pause** / **Resume** / **Stop**,
**Check for changes** (or **Index now** the first time), **Show in Folders**,
**Show in the file manager**, **Copy path** and **Remove from Catalog…**.

Removing a source says how many photos leave the Catalog. Your files, sidecars
and edits are not touched, and the folder stays in your Folders list.

### Folders in the Catalog

Every folder of a source shows how many photos are in it: the photos in it and
in all its folders below it while **Include subfolders** is on, only the ones
directly in it while it is off, as the grid will show them. Counts come from
the Catalog, never from the drive, so the tree opens at once and works with a
NAS asleep or unplugged. Above 99,999 a count is abbreviated (1.2M); hover it
for the whole number. A folder that is gone from the disk disappears from the
tree unless photos of it are still in the Catalog; a folder with no photos
shows 0, dimmed. A folder the index could not read shows a triangle.

While a source is being indexed its tree grows as folders are found, and the
counts rise in place, refreshed about every two seconds. Nothing opens by
itself, the rows you are looking at do not jump, and until the first folder is
found the row under the source says "Finding folders…". A source that has not
been indexed yet has no arrow.

The tree works from the keyboard like the Folders tree: Up, Down, Left, Right,
Home, End, Enter or Space to open a folder, `*`, the Menu key or Shift+F10, and
type-ahead (Left and Right swap in a right-to-left language). The folders you
have open are remembered between launches.

A folder opened from one tree is marked in the other with a small dot ("Also
open from Folders", "Also open from Catalog"), when the same folder is in both.
The row of the tree you opened it from is highlighted. A folder's menu has
**Show in Folders** (or, from a folder in Folders that is in the Catalog,
**Show in Catalog**), which only scrolls the other tree to it and opens what is
above it; it does not open the folder.

A folder's menu in the Catalog has **Open**, **Cull this folder…**,
**Statistics for this folder**, **Show in Folders**, **Show in the file
manager**, **Copy path**, and a grayed line saying which source it is part of.
A folder cannot be removed from the Catalog by itself, and photos cannot be
dragged into or out of Catalog folders: move photos in Folders and the Catalog
follows on its next check.

The first-index card for the source you last clicked stays at the bottom of the
card while a long tree scrolls.

### Folding the card

Click the **Catalog** header (or focus it and press Enter or Space) to fold or
open the card; Alt-click folds the others. Immage remembers it between
launches, and it starts open. A folded card still shows the work in its header,
in the space between the title and the buttons, so nothing moves when a job
starts or ends:

- a ring and **Indexing 1,240 / 8,000** (or **Thumbnails ...**, or **Finding
  photos** while the folders are still being listed), counted across every
  source that is being indexed;
- two bars and **Paused** when the work is paused;
- a triangle when any source needs attention, beside the work if there is some.

In a narrow panel the text shortens to the count, then to a percent, then to the
ring alone; hover it for the full line.

## A source that is offline

When a drive or share is not connected, its row is dimmed, with an empty ring.
Its photos stay counted in the Catalog and its whole folder tree stays
browsable: the tree and the counts come from the Catalog, so you can find a
folder and see how many photos it holds without the drive. Its folders are
dimmed too, with the tooltip "Not connected. Showing what the Catalog knows."

Clicking an offline folder selects it and shows a card under the tree with the
number of photos, when the source was last checked, and **Check again**, which
looks at the connections once more and opens the folder if its drive is back.
Its photos are not shown in the grid yet. **Cull this folder…**,
**Statistics for this folder**, **Check for changes** and **Show in the file
manager** wait until it is back. Nothing is changed or queued while it is away.

## Folder menu

Right-click a folder in **Folders**:

- **Cull this folder…** opens it in the grid and starts
  [Auto cull](library.md#auto-cull) for it.
- **Statistics for this folder** opens [Statistics](statistics.md) with the
  **This folder** scope.
- **Add to Catalog…** for a folder that is not in the Catalog. For one that is,
  **Show in Catalog** (which opens the Catalog's tree to the folder) and, for a
  source's own folder, **Remove from Catalog…** instead.

## Preferences

**Preferences > General > Catalog** shows how many sources and photos the
Catalog holds and how many are online, what the Catalog and its thumbnails use
on disk (**Clean up thumbnails** removes thumbnails nothing needs), and
**Rebuild catalog**. Rebuild reads every photo's file information again. It asks
twice: the first click shows "Rebuild N photos?" with **Cancel** focused, and
only **Yes, rebuild** starts it. It is for a Catalog that looks wrong; for a NAS
it can take a long time. Your files and sidecars are not touched. It is refused
while an index is running.

The **Local photo cache** under Preferences > Performance, which Find and
Statistics still read, is separate from the Catalog for now.

## Not in this version

- Showing an offline source's cached photos and thumbnails in the grid (its folders and counts are browsable now), and rating or labeling them while it is away.
- Collections (saved searches and albums) beside Sources. Saved Find filters
  keep working as before.
- Checking sources automatically when Immage starts, a speed test before adding
  a NAS, and a history of past indexes. Check a source from its menu.
- Relinking a source to a new path from the app (the command line does it).

The command line, `immage catalog`, does the same indexing without a window: see
[Catalog command line](catalog-cli.md).
