# NAS benchmark (read-only)

`nas_bench.py` measures how a photo library behaves on a network mount (SMB/cifs, NFS) or a
local disk, to size the catalog's discovery, metadata and thumbnail pipelines. Python 3
standard library only; `exiftool` is optional (used on local copies of header bytes).

## Safety rules (enforced in code)

- Strictly read-only on the measured tree: `stat`, `scandir`, `open(O_RDONLY|O_NOATIME)`,
  `pread`, `posix_fadvise`. No create, write, rename, delete, chmod, touch or lock files.
  Scratch files go to a local `--tmp-dir` only, and are deleted.
- Concurrency is limited to 1, 2, 4 or 8 (`--max-conc`), a global rate limit
  (`--max-ops-per-sec`, default 2500) applies to every operation, and the process runs at
  `nice 10` and ionice idle class when available. Run it as
  `nice -n 10 ionice -c3 python3 nas_bench.py ...` as well.
- Emergency stop (the run ends, the partial results are written, nothing is retried):
  window p95 latency above 5x the baseline p95 (with a 30 ms floor), error rate above 2%,
  10 consecutive errors, the mount disappearing or changing, or the byte cap (default 2 GB).
- Budgets: each experiment has its own time budget; the whole run is capped at 45 minutes.
- Headers only: reads are at most 2 MB per file, except experiment 7, which reads up to
  400 MB in 8 MB chunks from a few large files.
- No names: reports contain counts, extensions, hashed folder labels and the share's
  top-level name. Error messages are reduced to errno names.

## Usage

```
# see what would happen, touching nothing
python3 nas_bench.py --dry-run --root /mnt/nas/Photos

# measure a mount
nice -n 10 ionice -c3 python3 nas_bench.py --root /mnt/nas/Photos --label nas --out nas.json

# use the roots saved in Immage's settings that live under one mount
python3 nas_bench.py --roots-from-settings ~/.local/share/app.immage/settings.json \
    --mount-prefix /mnt/nas --label nas --out nas.json

# local baseline on a generated tree of a few hundred small files
python3 nas_bench.py --generate-local /tmp/nas-local-tree --label local --out local.json

# quick smoke run with shorter budgets, or only some experiments
python3 nas_bench.py --root /mnt/nas/Photos --time-scale 0.3 --only 1,2,3
```

Exit code 0 is a complete run, 2 a usage error, 3 an emergency stop (see `guard.emergency_stop`
in the JSON).

## Experiments

| id | what |
|---|---|
| 1 | mount facts (fs type, host hash, options, autofs), first and later access, share usage |
| 2 | stat latency p50/p95/p99 after attribute-cache expiry and cached repeats; sets the latency baseline |
| 3 | bounded BFS and random descents: directory count, depth, entries per directory, listing throughput by directory size, concurrency scaling |
| 4 | cold stat throughput on a sample (default 20,000 files) at 1/2/4/8 threads; scandir d_type only vs scandir + DirEntry.stat |
| 5 | directory mtime vs the newest child mtime (read-only), timestamp granularity, attribute-cache behaviour |
| 6 | header reads of 64 KB, 256 KB, 1 MB (one open + pread) at 1/2/4/8; size distribution per extension |
| 6b | exiftool on local copies of 2 MB prefixes: smallest prefix with EXIF and with a complete embedded preview |
| 7 | 8/4/1 MB sequential and random chunk reads from large files |
| 8 | extrapolation to a library of `--library-tb-low/mid/high` TB (no I/O) |

## Reading the output

`results.json` has `experiments.<id>` with the numbers, `8_estimate` with the extrapolations
and `guard` with totals (ops, errors, bytes, emergency stop). Latencies are in milliseconds,
rates per second. Cells in experiment 6 are named `<size>k_c<concurrency>`.

Things to keep in mind:

- The kernel attribute cache (`actimeo`) makes back-to-back stats of the same object nearly
  free; the tool waits for it to expire between rounds and reports cached repeats separately.
- Page-cache is dropped per file with `posix_fadvise(DONTNEED)`, but server-side caches and
  NAS disk read-ahead cannot be bypassed; repeated runs on the same files read warmer.
- The directory sample is a bounded BFS plus random descents: it favours shallow
  directories, so treat directory and file-count extrapolations as roughly +-2x.
- Experiment 5 cannot prove that a directory's mtime changes when a child is added; that
  needs a write. See the manual procedure in the baseline report.
- Local synthetic files are not real RAWs, so experiment 6b is only meaningful on real photos.

## Measuring Immage's own catalog: `immage catalog bench`

`nas_bench.py` measures the share. `immage catalog bench` measures Immage's real code paths
(folder listing, file stat, the metadata head reads and their 64 KB, 256 KB, 1 MB ladder,
embedded preview reads, SQLite batch writes) on a folder, in one run, and writes aggregated
numbers. Use it on a release build: a debug build is several times slower and the report says so.

```
# once, from the repository root (about as long as any release build)
cd app/src-tauri && cargo +1.98.0 build --release --locked

# a folder or a source (id or name) already added in the app; reads only
nice -n 10 ionice -c3 ./target/release/immage catalog bench /mnt/synology/Photos \
    --concurrency 2 --max-files 5000 --duration 300 \
    --out ~/nas-bench/ds220-run1
```

`--out` is a base name: it writes `ds220-run1.json` and `ds220-run1.txt` (the same numbers, readable).
Without it the files go to the current folder as `immage-catalog-bench-<time>`. The report path
must not be inside the measured folder (it is refused). `--scratch <local folder>` moves the scratch SQLite
database off the system temp folder (a network folder is refused).

| option | default | meaning |
|---|---|---|
| `--concurrency N` | 2 on a network share, 4 local; at most 8 | workers at the start and the most it will use |
| `--max-files N` | 5000 | photos to find and read; discovery stops at this many |
| `--duration S` | 300 | overall time budget: discovery gets up to 40%, metadata until 85%, previews the rest |
| `--latency-limit-ms MS` | 500 | window p95 above this halves the workers and pauses them for a second |
| `--preview-sample N` | 30 | embedded previews read per format |
| `--json` | off | print the JSON on stdout as well |

Safety: nothing is created, written or locked on the measured folder; files are opened read-only
(no access-time update) and only their first bytes and the embedded preview are read. The
user's catalog is not opened. Slow answers make it back off, not stop; it ends early only on Ctrl-C
(the report is still written, exit 130), when 64 reads in a row fail (`ended: error-storm`), or
when the time budget is used. Exit 4 means the folder does not answer.

The report holds counts, sizes, latencies and extensions. It has no file or folder names. It does
contain the mount's host and share name and the whitelisted mount options (`vers`, `cache`,
`rsize`, `actimeo`, ...), never user names, addresses or passwords.

### Reading the report

- `discovery`: `dirs_per_s` and `entries_per_s` are the listing rate. `list_latency_ms` is the time
  of one directory listing; `list_latency_by_dir_size_ms` separates small and huge folders, where
  the second is what a flat folder of thousands of photos costs. `walk_complete: false` means the
  walk was cut by `--max-files` or `--duration`, so the counts are a sample.
- `stat`: one stat per sampled file and directory, one at a time. The files were listed moments
  earlier, so an `actimeo` attribute cache can answer some without the network; compare with
  `nas_bench.py` experiment 2, which waits for the cache to expire.
- `metadata.by_format`: per format (cr3, cr2, nef, arw, raf, dng, heic, jpg, other-raw, other),
  `read_latency_ms` is one head read; `size_read_bytes` is how much was read per file in the end;
  `bytes_asked_by_parser` is what the parser needed when it wanted more than the window;
  `ladder_finished_at` counts files that finished at 64 KB, 256 KB, 1 MB (`retried` is how many needed
  a second read); `parse_failures` are `unsupported` (not a format Immage reads), `corrupt` and
  `read_errors`. A format with many retries is the one to give a bigger first read
  (`--first-read` of `catalog metadata`).
- `thumbnails.by_format`: the cost of the thumbnail step's single ranged read of the embedded preview:
  bytes, latency and `ms_per_mb`. `valid_jpeg` below `reads` means the recorded preview location
  was wrong. JPEGs with no usable embedded preview are whole-file reads in the real step; their
  sizes are in `discovery.by_format` (`size_p50`, `size_p95`).
- `concurrency` and `safety`: `backoffs` is how often the latency limit tripped, `lowest` the fewest
  workers used, `events` the timeline of changes, `window_p95_peak_ms` the worst window. Many
  backoffs mean the share was the bottleneck at that concurrency; rerun with a lower `--concurrency`.
- `timeline`: dirs/s, entries/s, files/s and MB/s per 10 seconds, with CPU % and RSS. A falling
  files/s with a steady CPU means the share slowed (or a cache emptied); a steady curve means the run
  is representative.
- `sqlite`: write time per batch of 500 rows, the largest WAL file seen and the database size. The
  scratch database lives on a local disk, so this is the catalog's own write cost, not the network.
- `process`: CPU seconds (user, system), average CPU %, peak RSS. `network.payload_bytes_read` is what
  Immage asked for; `system_rx_bytes_delta` is the machine's total receive counter over the run
  (all interfaces except loopback), so it includes SMB overhead and other traffic.
- `errors`: counts by kind (`timeout`, `denied`, `not-found`, `io (os error N)`) only.

Run it twice: the second run shows what a warm cache and a warm NAS give, which is what the catalog's
rescans see.
