# Catalog command line

`immage catalog` builds and checks Immage's catalog from a terminal. It runs
without opening a window, so it works over SSH and from cron. It finds photos
(folder, name, size and modified time) and then reads their metadata (camera,
lens, exposure, time, size, GPS, rating, labels and keywords). It does not make
thumbnails yet, and the catalog is not shown in the app yet.

The catalog is a database in Immage's data folder (`catalog.sqlite`). It lives
on your own disk, never on a network share. Immage never changes, moves or
deletes your photos or their sidecars: scanning only lists folders.

```
immage catalog add-source /mnt/photos/2023 --name "2023 Japan"
immage catalog scan 1 --metadata
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
- `remove-source <id|name> --yes` forgets a source and everything the catalog
  knows about it. Your files are not touched.

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

`rescan <id|name>` is `scan` for a source you have scanned before.

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

## Status and checks

- `status [<id|name>]` shows the catalog's size, the sources, how many photos
  have their metadata read, are waiting, failed or are not understood, and what
  needs attention; `--watch` repeats every 2 seconds. Exit 5 means something needs
  attention.
- `verify` checks the database. `--deep` also compares every recorded size and
  time with the files (this reads the volume). `--fix` repairs catalog-side
  problems only.

## For scripts

`--json` makes stdout one JSON document per result. With `--progress`, progress
events are one JSON object per line on stderr. `--db <file>` or
`--catalog <folder>` use another catalog. Exit codes:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Failure |
| 2 | Usage error (also: an overlapping source, a missing `--yes`) |
| 3 | Catalog problem: open in Immage, made by a newer Immage, or damaged |
| 4 | Every requested source is unreachable |
| 5 | Finished, but some folders were incomplete, some files unreadable (or, for `metadata`, some photos failed to read), or a check found problems |
| 130 | Interrupted; progress is saved, rerun to continue |

While the Immage app has the catalog open, the commands that only read
(`list-sources`, `status`, `verify` without `--fix`) still work. The ones that
write ask you to close the app first (exit 3).
