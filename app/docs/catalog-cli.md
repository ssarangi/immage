# Catalog command line

`immage catalog` builds and checks Immage's catalog from a terminal. It runs
without opening a window, so it works over SSH and from cron. It finds photos
(folder, name, size and modified time) and then reads their metadata (camera,
lens, exposure, time, size, GPS, rating, labels and keywords), and then makes
small thumbnails with the fewest possible reads. The catalog is not shown in the
app yet, but the thumbnails it makes are already used by the library grid.

The catalog is a database, `catalog.sqlite`, in Immage's data folder
(`$XDG_DATA_HOME/app.immage`, by default `~/.local/share/app.immage`, on Linux, `~/Library/Application Support/app.immage`
on macOS, `%APPDATA%\app.immage` on Windows). It must be on a local disk: SQLite's
write-ahead log does not work on network shares. Immage always refuses to open or
create the catalog on a network file system (NFS, SMB/CIFS, sshfs, FUSE mounts such
as rclone and gvfs, and automounts that have not mounted anything yet), whether the
path comes from `--db`, `--catalog` or the default, and exits 2 with a message
naming the share. On Linux, a path under `/mnt` or `/media` whose mount cannot be
determined is refused too. There is no option to override this. Your photos can be
on a network share; only the catalog cannot.
Immage never changes, moves or deletes your photos or their sidecars: scanning
lists folders and reading metadata opens files for reading only.

Run it as `immage catalog ...` with the same program that opens the app. Nothing
is sent anywhere.

```
immage catalog add-source /mnt/photos/2023 --name "2023 Japan"
immage catalog scan 1 --metadata
immage catalog thumbnails 1
immage catalog status
```

## Sources

A source is a folder on a volume. Immage remembers the volume itself (the server
and share of a network mount, the file system of a drive), not the path it is
mounted at. If your NAS is mounted at `/mnt/nas` today and `/media/nas/photos`
tomorrow, the source follows it.

- `add-source <path>` adds a folder and says how it was detected:

  ```
  Folder   /mnt/photos/2023
  Storage  Network share, SMB (nas.local, photos)
  Test     listing 4,200 files/s: fast (read only, 3 s); reading photos is not measured yet
  Added source 1 "2023 Japan". Policy: on launch. Immage never changes your files.
  Run: immage catalog scan 1
  ```

  Options: `--name`, `--policy launch|every=6h|manual`, `--no-subfolders`,
  `--index` (scan straight away), `--skip-test`, `--allow-offline`. A folder
  that does not answer exits 4 unless you give `--allow-offline`, which adds it
  to be indexed when it appears. A folder that overlaps an existing source is
  refused.
- `list-sources` shows each source, its state and how many photos it holds.
- `remove-source <id|name>` forgets a source and everything the catalog knows
  about it, after telling you how many photos and folders that is. On a terminal
  it asks `[y/N]`; in a script, give `--yes`, or it stops with exit 2. Your files
  are not touched.

Network mounts behind a systemd automount (`x-systemd.automount`) are
recognised: while nothing is mounted a source shows as `asleep`, not offline,
and a scan wakes the mount.

## Scanning

`scan <id|name>...` or `scan --all` walks the source and records what is there.
Progress is written to stderr: one updating line on a terminal, one line every
10 seconds otherwise. A summary goes to stdout:

```
2023 Japan: checked 12,060 photos in 38 s, 14 new, 3 changed, 0 missing.
```

How a scan stays safe on a slow or flaky share:

- It lists two folders at a time on a network share, each with a time limit. A
  folder that does not answer is recorded as not answering. It is never read as
  empty.
- Photos are marked missing only when the whole walk finished with no problems,
  the volume was online at the start and the end, and it was the same mount
  throughout. Otherwise the scan ends `incomplete` and names the folders:
  `incomplete: Raw/2021-08 (timed out after 12 s); nothing marked missing`.
- A folder that lists as empty above photos Immage already knows about is not
  believed (a share that has gone away can look like that).
- Links to folders are noted and not followed. Hidden files and folders are
  skipped. A file whose name is not valid text is counted and skipped.
- Ctrl-C stops the scan and keeps its place. Run the same command again to
  continue it. If the share goes away mid-scan, the same happens.

Options: `--light` skips folders whose modified time has not changed (faster,
but a folder's time is not always updated on a NAS, so use it for quick checks
and keep an occasional full scan), `--concurrency <n>` sets the number of
folders listed at once.

`rescan <id|name>` is `scan` for a source you have scanned before. `scan --thumbnails`
runs the metadata and thumbnail steps after the scan (`--no-thumbnails` is accepted and
is the default); `scan --metadata-only` runs just the metadata step. The commands `rebuild`, `backup` and `restore` are
reserved: they report that they are not available in this version (exit 1).

## Reading metadata

`metadata <id|name>...` or `metadata --all` (and `scan --metadata`, which runs it
after the scan) reads the start of every photo the scan found new or changed, and
the small XMP sidecar beside it, and records the result in the catalog:

```
2023 Japan: read 12,074 photos in 4 min 31 s (45/s), 0 failed, 12 not understood.
2023 Japan: 3 renamed or moved photos were matched to their old entries.
```

What it records: camera make and model, body serial, lens, focal length (and the
35 mm equivalent), aperture, ISO, shutter speed, exposure compensation, the time
taken (with sub-seconds and the time zone offset when the file has one),
orientation, pixel size (and a RAW's sensor size), where the embedded preview
sits in the file (for the thumbnail step), GPS position and altitude, software,
and from the sidecar the star rating, reject flag, colour label, keywords and
whether darktable edits exist. JPEG, TIFF, every TIFF-based RAW (ARW, NEF, DNG,
CR2, ORF, RW2, PEF, SRW, ...), Canon CR3, HEIC, HEIF, AVIF, Fujifilm RAF and
MP4/MOV are read; other formats are marked "not understood" (see below).

How it stays gentle on a slow share:

- It reads only the first 64 KB of a file, and more (256 KB, then 1 MB) only for
  the rare file whose format needs it. Files already read and unchanged are
  never opened again; a rescan that changed nothing reads nothing.
- Two files are read at a time on a network share (four on a local drive;
  `--concurrency <n>` overrides). Every read has a 5 second limit. When reads
  get slow, Immage reads fewer files at once and speeds up again when the share
  recovers (`--no-adaptive` turns this off).
- Reading never changes the volume. A read that fails is recorded for that photo
  only (`failed`, with the reason) and is tried again by the next run
  (`--skip-failed` leaves failed photos alone). A read error never marks a source
  offline or a photo missing. If the share stops answering altogether, the run
  stops and keeps its place; nothing from that moment is recorded as failed.
- A sidecar is read only when it is under 1 MB. If only a sidecar changed (you
  rated a photo in darktable), only the sidecar is read again.
- Photos whose format Immage cannot read are marked `unparsed`, not failed, and
  are not retried until the file changes; they are kept for a later fallback.
- Ctrl-C keeps the progress; run the command again to continue.

Renamed and moved photos: after reading, a photo that vanished and a new photo in
the same source that have the same size, modified time, first 64 KB, capture time
and body serial are the same photo. Immage keeps the old entry (so its tags and
analyses stay) and gives it the new location. A copy (the original is still there)
is not a move. If two new files could both be the old one, nothing is changed.
Photos moved to a different source are not matched.

Options: `--concurrency <n>`, `--first-read <KB>` (a bigger first read, 64 to
1024, which also finds a RAW's sensor size in files that keep it far from the
start), `--skip-failed`, `--no-sidecars`, `--no-relink`, `--no-adaptive`.

## Making thumbnails

`thumbnails <id|name>...` or `thumbnails --all` (and `scan --thumbnails`) makes a small
JPEG, 480 pixels on the long edge, for every photo whose metadata has been read and that
has no current thumbnail:

```
2023 Japan: made 11,980 thumbnails in 9 min 12 s (21/s): 11,402 from previews, 9 from Exif thumbnails, 569 from whole JPEGs; 0 failed, 94 left for the app.
```

Where the picture comes from, cheapest first:

- **A RAW's embedded preview** (ARW, DNG, NEF, CR2, CR3, RAF): one read of exactly that part
  of the file, which the metadata step recorded. For a Sony ARW that is about 400 KB, once.
  The 40 MB of raw data are never read.
- **A JPEG's Exif thumbnail**, when it is at least 256 pixels on the long edge: one read of
  the first 80 KB.
- **The JPEG itself**, read whole, when it has no usable Exif thumbnail (most cameras store
  one of 160 pixels). This is the expensive case, a 19 MB JPEG is 19 MB over the network,
  so `--whole-jpegs no|idle|yes` decides: `no` never reads one (files under 768 KB are
  still read, as that is no dearer than a second try), `idle`, the default, reads them
  whenever the app is not being used for browsing, which on the command line is always,
  and `yes` always does. Photos skipped for this reason are listed as "left for the app"
  and made when the setting allows.
- **Formats only the app's engine reads** (HEIC, TIFF, PNG) and previews that do not
  decode are recorded, not dropped: "left for the app" or "failed".

The picture is turned upright by the photo's orientation, and written once into the catalog's
thumbnail store (`catalog-thumbs` beside `catalog.sqlite`, 3 GB at most, `--store <folder>`
and `--store-limit <MB>`), one file per photo, so a renamed or moved photo keeps its thumbnail.
It is also linked into the library's thumbnail cache (`--cache <folder>`, the app's cache
folder by default) under the name the grid looks for, so Immage's grid shows these thumbnails
with no work of its own. The thumbnails are the camera's own previews, so they can look a
little different from Immage's rendering until you edit the photo, which Immage then renders as
usual. The library cache has its own size limit (Preferences > Cache, 1 GB by default, read
from your settings); linking a very large library evicts older entries from it, and a
thumbnail evicted from it is found again in the store. `--no-publish` skips the link.

How it stays gentle:

- Two files are read at a time on a network share (four on a local drive,
  `--concurrency <n>`), each read with a 5 second limit, fewer at once when the share
  slows down (`--no-adaptive` turns this off). Pictures are decoded and resized on other
  threads (`--decode-workers <n>`, by default all cores but one), at low CPU and disk
  priority.
- Newest folders first. Photos that already have a current thumbnail, and unchanged
  photos, are never opened again.
- A photo that cannot be read or decoded is recorded as failed and tried again after an hour,
  a day, then a week (`--skip-failed` leaves failed photos alone). A read error never marks a
  source offline or a photo missing. If the volume stops answering, the run stops with its
  progress kept; if the disk is nearly full (under 5 GB free) it stops and says so. Ctrl-C
  keeps the progress.
- Thumbnails of photos that are marked missing (an offline share, or deleted originals) are
  kept. When the store passes its limit, thumbnails of photos that are still present go first
  (they can be made again), then those of missing photos, the oldest first.

`thumbnails --gc` reads no photo: it links again whatever the cache lost (after a rename, a
move or an eviction), removes thumbnail files nothing refers to and trims the store.

Options: `--size <px>` (64 to 2048), `--quality <30-95>`, `--concurrency <n>`,
`--decode-workers <n>`, `--whole-jpegs no|idle|yes`, `--skip-failed`, `--no-publish`,
`--no-adaptive`, `--missing-only` (the default: only photos without a current thumbnail), `--gc`, `--store`, `--store-limit`, `--cache`, `--cache-limit <MB>`.

## Reserved options

`--keep-annotations` (remove-source), `--read-only` and `--regenerate` are reserved
for later versions. For now they stop with "not available yet" and exit 2.
`--no-color` is accepted and does nothing, because the output is never colored.

## Status and checks

- `status [<id|name>]` shows the catalog's size, the sources, how many photos
  have their metadata read, are waiting, failed or are not understood, how many
  have a thumbnail, are waiting, failed or left for the app, how many thumbnails are
  kept for missing photos, and what needs attention; `--watch` repeats every 2 seconds. Exit 5 means something needs
  attention.
- `verify` checks the database. `--deep` also compares every recorded size and
  time with the files (this reads the volume). `--fix` repairs catalog-side
  problems only.

## For scripts

`--quiet` and `--verbose` change how much is written to the terminal. `--json` makes stdout one JSON document per result. With `--progress`, progress
events are one JSON object per line on stderr. `--db <file>` or
`--catalog <folder>` use another catalog. Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Failure |
| 2 | Usage error (also: an overlapping source, a missing `--yes`) |
| 3 | Catalog problem: open in Immage, made by a newer Immage, or damaged |
| 4 | Every requested source is unreachable |
| 5 | Finished, but some folders were incomplete, some files unreadable (or, for `metadata` and `thumbnails`, some photos failed), or a check found problems |
| 130 | Interrupted; progress is saved, rerun to continue |

While the Immage app has the catalog open, the commands that only read
(`list-sources`, `status`, `verify` without `--fix`) still work. The ones that
write ask you to close the app first (exit 3).
