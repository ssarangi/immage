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
