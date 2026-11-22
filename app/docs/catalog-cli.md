# Catalog command line

`immage catalog` builds and checks Immage's catalog from a terminal. It runs
without opening a window, so it works over SSH and from cron. This first
version finds photos and records their folder, name, size and modified time. It
does not read photo metadata or make thumbnails yet, and the catalog is not
shown in the app yet.

The catalog is a database in Immage's data folder (`catalog.sqlite`). It lives
on your own disk, never on a network share. Immage never changes, moves or
deletes your photos or their sidecars: scanning only lists folders.

```
immage catalog add-source /mnt/photos/2023 --name "2023 Japan"
immage catalog scan 1
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

## Status and checks

- `status [<id|name>]` shows the catalog's size, the sources and what needs
  attention; `--watch` repeats every 2 seconds. Exit 5 means something needs
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
| 5 | Finished, but some folders were incomplete, some files unreadable, or a check found problems |
| 130 | Interrupted; progress is saved, rerun to continue |

While the Immage app has the catalog open, the commands that only read
(`list-sources`, `status`, `verify` without `--fix`) still work. The ones that
write ask you to close the app first (exit 3).
