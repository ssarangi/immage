#!/usr/bin/env python3
"""Read-only benchmark for a photo library on a network mount (SMB/cifs, NFS) or local disk.

Standard library only (exiftool is optional and used on local copies of header bytes).
It never writes, renames, creates, touches or locks anything under the roots it measures.
It records no file or folder names: folders are numbered, extensions are reported,
everything else is aggregates. See README.md in this directory.
"""
import argparse, base64, collections, errno, hashlib, itertools, json, math, os, random
import shutil, subprocess, sys, tempfile, threading, time

READ_EXTS = ["jpg", "jpeg", "png", "heic", "heif", "avif", "tif", "tiff", "dng", "cr2", "cr3",
             "crw", "nef", "nrw", "arw", "sr2", "srf", "orf", "raf", "rw2", "pef", "srw",
             "3fr", "iiq", "erf", "mef", "mrw", "x3f", "gpr"]
PREFIX_LADDER = [16384, 32768, 65536, 131072, 262144, 524288, 1048576, 2097152]
ALLOWED_CONC = (1, 2, 4, 8)
MB = 1024 * 1024


# ---------------------------------------------------------------- helpers
def pct(vals, p):
    if not vals:
        return None
    s = sorted(vals)
    k = (len(s) - 1) * p / 100.0
    lo, hi = int(math.floor(k)), int(math.ceil(k))
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def lat_stats(secs):
    """Latency summary in milliseconds from a list of seconds."""
    if not secs:
        return {"n": 0}
    ms = [x * 1000.0 for x in secs]
    return {"n": len(ms), "mean": round(sum(ms) / len(ms), 3), "p50": round(pct(ms, 50), 3),
            "p95": round(pct(ms, 95), 3), "p99": round(pct(ms, 99), 3), "max": round(max(ms), 3)}


def size_stats(sizes):
    if not sizes:
        return {"n": 0}
    return {"n": len(sizes), "median": int(pct(sizes, 50)), "p95": int(pct(sizes, 95)),
            "mean": int(sum(sizes) / len(sizes)), "total_bytes": int(sum(sizes))}


def ext_of(name):
    i = name.rfind(".")
    return name[i + 1:].lower() if i > 0 else ""


def ecode(e):
    return errno.errorcode.get(getattr(e, "errno", None) or 0, "EOTHER")


def log(msg):
    print(time.strftime("%H:%M:%S ") + msg, flush=True)


def read_mountinfo():
    out = []
    try:
        with open("/proc/self/mountinfo") as f:
            for line in f:
                p = line.split()
                dash = p.index("-")
                out.append({"mount_point": p[4].replace("\\040", " "), "opts": p[5], "fstype": p[dash + 1],
                            "source": p[dash + 2], "super_opts": p[dash + 3] if len(p) > dash + 3 else ""})
    except OSError:
        pass
    return out


def mounts_for(path):
    """All mountinfo entries stacked at the longest mount point containing path (autofs + cifs)."""
    path = os.path.abspath(path)
    best, ents = "", []
    for m in read_mountinfo():
        mp = m["mount_point"]
        if (path == mp or path.startswith(mp.rstrip("/") + "/")) and len(mp) >= len(best):
            if len(mp) > len(best):
                ents = []
            best = mp
            ents.append(m)
    return best, ents


# ---------------------------------------------------------------- safety guard
class StopRun(Exception):
    pass


class Limiter:
    def __init__(self, rate):
        self.interval = 1.0 / rate if rate > 0 else 0
        self.lock = threading.Lock()
        self.next = 0.0

    def wait(self):
        if not self.interval:
            return
        with self.lock:
            now = time.monotonic()
            t = max(now, self.next)
            self.next = t + self.interval
        if t > now:
            time.sleep(t - now)


class Guard:
    """Every operation on the measured tree goes through before()/after(). It enforces the global
    rate limit, the byte cap, the error-rate stop, the latency stop and the mount-disappeared stop."""

    def __init__(self, a, mount_point, mount_fstype):
        self.a = a
        self.stop = threading.Event()
        self.reason = None
        self.lock = threading.Lock()
        self.limiter = Limiter(a.max_ops_per_sec)
        self.bytes = 0
        self.ops = 0
        self.errs = 0
        self.errcodes = collections.Counter()
        self.win = collections.defaultdict(lambda: collections.deque(maxlen=200))
        self.errwin = collections.deque(maxlen=200)
        self.baseline = {}
        self.mount_point = mount_point
        self.mount_fstype = mount_fstype
        self.last_mount_check = 0.0

    def trip(self, reason):
        with self.lock:
            if not self.stop.is_set():
                self.reason = reason
                self.stop.set()
                log("EMERGENCY STOP: " + reason)

    def set_baseline(self, kind, p95_s):
        self.baseline[kind] = p95_s

    def before(self):
        if self.stop.is_set():
            raise StopRun()
        self.limiter.wait()
        now = time.monotonic()
        if self.mount_point and now - self.last_mount_check > 2.0:
            self.last_mount_check = now
            _, ents = mounts_for(self.mount_point)
            if not ents or ents[-1]["fstype"] != self.mount_fstype or ents[-1]["mount_point"] != self.mount_point:
                self.trip("mount disappeared or changed")
                raise StopRun()

    def after(self, kind, lat, ok, nbytes=0, err=None):
        lat_bad = None
        with self.lock:
            self.ops += 1
            self.bytes += nbytes
            self.errwin.append(0 if ok else 1)
            if not ok:
                self.errs += 1
                self.errcodes[err or "?"] += 1
            w = self.win[kind]
            w.append(lat)
            over_bytes = self.bytes > self.a.max_bytes
            err_rate = sum(self.errwin) / len(self.errwin) if len(self.errwin) >= 50 else 0
            consecutive = len(self.errwin) >= 10 and sum(list(self.errwin)[-10:]) == 10
            b = self.baseline.get(kind)
            if b is not None and len(w) >= 100:
                p95 = pct(list(w), 95)
                lim = max(self.a.latency_factor * b, self.a.latency_floor_ms / 1000.0)
                if p95 > lim:
                    lat_bad = "%s p95 %.1f ms > %.1f ms (%.0fx baseline %.2f ms)" % (
                        kind, p95 * 1000, lim * 1000, self.a.latency_factor, b * 1000)
            codes = dict(self.errcodes)
        if over_bytes:
            self.trip("byte cap %d reached" % self.a.max_bytes)
        if err_rate > self.a.max_error_rate:
            self.trip("error rate %.1f%% over last window (%s)" % (err_rate * 100, codes))
        if lat_bad:
            self.trip("latency: " + lat_bad)
        if consecutive:
            self.trip("10 consecutive errors")

    def timed(self, kind, fn, *args, nbytes=None):
        """Run fn(*args) as one guarded, timed operation. Returns (ok, result, latency_s, errname)."""
        self.before()
        t = time.perf_counter()
        try:
            r = fn(*args)
            ok, err = True, None
        except OSError as e:
            r, ok, err = None, False, ecode(e)
        lat = time.perf_counter() - t
        nb = nbytes(r) if (ok and nbytes) else 0
        self.after(kind, lat, ok, nb, err)
        return ok, r, lat, err


class Ctx:
    def __init__(self, a, guard, state):
        self.a, self.g, self.state = a, guard, state
        self.deadline = None
        self.salt = os.urandom(8)
        self.rng = random.Random(a.seed)

    def expired(self):
        return self.g.stop.is_set() or (self.deadline is not None and time.monotonic() > self.deadline)

    def budget(self, secs):
        self.deadline = time.monotonic() + secs * self.a.time_scale

    def hid(self, s):
        return hashlib.sha256(self.salt + s.encode("utf-8", "surrogateescape")).hexdigest()[:8]


def pool_map(ctx, items, fn, conc):
    """Run fn(item) over items with conc worker threads; stops on deadline/guard stop. Returns (results, seconds)."""
    it = iter(items)
    lock = threading.Lock()
    out = []
    dl = ctx.deadline

    def worker():
        while True:
            if ctx.g.stop.is_set() or (dl is not None and time.monotonic() > dl):
                return
            with lock:
                try:
                    x = next(it)
                except StopIteration:
                    return
            try:
                r = fn(x)
            except StopRun:
                return
            with lock:
                out.append(r)

    ths = [threading.Thread(target=worker, daemon=True) for _ in range(conc)]
    t0 = time.perf_counter()
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    return out, time.perf_counter() - t0


# ---------------------------------------------------------------- listing primitive
def list_dir(ctx, path, kind="readdir"):
    """scandir with d_type only (no per-file stat). Returns a record (no names are ever reported)."""
    def go():
        files, dirs, other = [], [], 0
        with os.scandir(path) as it:
            for e in it:
                try:
                    if e.is_dir(follow_symlinks=False):
                        dirs.append(e.name)
                    elif e.is_file(follow_symlinks=False):
                        files.append(e.name)
                    else:
                        other += 1
                except OSError:
                    other += 1
        return files, dirs, other
    ok, r, lat, err = ctx.g.timed(kind, go)
    if not ok:
        return {"ok": False, "err": err, "lat": lat, "path": path}
    files, dirs, other = r
    return {"ok": True, "lat": lat, "path": path, "files": files, "dirs": dirs, "other": other,
            "n": len(files) + len(dirs) + other}


def dir_bucket(n):
    return "small(<10)" if n < 10 else ("medium(10-200)" if n <= 200 else "large(>200)")


def linfit(xs, ys):
    n = len(xs)
    if n < 3:
        return None
    mx, my = sum(xs) / n, sum(ys) / n
    sxx = sum((x - mx) ** 2 for x in xs)
    if sxx == 0:
        return None
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sxx
    return {"a_s": my - b * mx, "b_s_per_entry": b}


# ---------------------------------------------------------------- experiment 1
def exp1_mount(ctx, roots):
    res = {"roots": len(roots)}
    path = roots[0]
    mp, ents = mounts_for(path)
    res["mount_point_found"] = bool(mp)
    res["stack"] = [{"fstype": m["fstype"]} for m in ents]
    for i, m in enumerate(ents):
        src = m["source"]
        if src.startswith("//") or (":" in src and m["fstype"].startswith("nfs")):
            s2 = src.lstrip("/")
            host = s2.split("/")[0].split(":")[0]
            parts = s2.split("/")
            res["stack"][i]["source_host_hash"] = hashlib.sha256(host.encode()).hexdigest()[:10]
            res["stack"][i]["share_name"] = parts[1] if len(parts) > 1 else ""  # top-level share name only
            res["stack"][i]["subpath_depth"] = max(0, len(parts) - 2)
        res["stack"][i]["options"] = (m["opts"] + "," + m["super_opts"]) if m["fstype"] in ("cifs", "smb3", "nfs", "nfs4") else m["opts"]
    res["behind_autofs"] = any(m["fstype"] == "autofs" for m in ents)
    top = ents[-1] if ents else None
    res["fstype"] = top["fstype"] if top else None
    res["is_network"] = bool(top and top["fstype"] in ("cifs", "smb3", "nfs", "nfs4", "fuse.sshfs"))
    first = ctx.g.timed("stat", os.stat, path)
    res["first_access_stat_ms"] = round(first[2] * 1000, 3)
    res["first_access_ok"] = first[0]
    later = []
    for _ in range(5):
        time.sleep(1.2)
        r = ctx.g.timed("stat", os.stat, path)
        later.append(r[2])
    res["later_access_stat_ms"] = [round(x * 1000, 3) for x in later]
    res["cold_automount_observed"] = bool(res["first_access_stat_ms"] > 500)
    res["note"] = ("The first access is only a true cold automount if the autofs mount was idle; with timeout=0 it never "
                   "expires, so a cold mount cannot be forced without umount (needs sudo).")
    try:
        sv = os.statvfs(path)
        res["share_total_gb"] = round(sv.f_blocks * sv.f_frsize / 1e9, 1)
        res["share_used_gb"] = round((sv.f_blocks - sv.f_bfree) * sv.f_frsize / 1e9, 1)
    except OSError:
        pass
    return res


# ---------------------------------------------------------------- experiment 2
def exp2_latency(ctx, roots):
    res = {}
    first = []
    for r in roots:
        rec = list_dir(ctx, r)
        if rec["ok"]:
            first.append(rec)
    dirs = list(roots)
    for rec in first:
        subs = rec["dirs"][:]
        ctx.rng.shuffle(subs)
        dirs += [os.path.join(rec["path"], s) for s in subs[:3]]
    dirs = dirs[:10]
    res["dirs_used"] = len(dirs)
    rounds = max(6, int(20 * ctx.a.time_scale))
    ctx.budget(120)
    cold, hot = [], []
    for _ in range(rounds):
        if ctx.expired():
            break
        time.sleep(1.2)  # let actimeo=1 attribute cache expire so each stat is a revalidating round trip
        for d in dirs:
            ok, _, lat, _ = ctx.g.timed("stat", os.stat, d)
            if ok:
                cold.append(lat)
        for d in dirs:  # immediate repeat: should come from the attribute cache
            ok, _, lat, _ = ctx.g.timed("stat_hot", os.stat, d)
            if ok:
                hot.append(lat)
    res["stat_after_cache_expiry_ms"] = lat_stats(cold)
    res["stat_immediate_repeat_ms"] = lat_stats(hot)
    if cold:
        b = pct(cold, 95)
        ctx.g.set_baseline("stat", b)
        res["baseline_p95_ms"] = round(b * 1000, 3)
        res["emergency_stop_threshold_ms"] = round(max(ctx.a.latency_factor * b, ctx.a.latency_floor_ms / 1000.0) * 1000, 1)
    small = sorted([r for r in first if r["ok"]], key=lambda r: r["n"])[:3]
    times = []
    for r in small:
        time.sleep(1.2)
        rec = list_dir(ctx, r["path"])
        if rec["ok"]:
            times.append((rec["n"], rec["lat"]))
    res["listing_round_trip"] = [{"entries": n, "ms": round(t * 1000, 3)} for n, t in times]
    res["notes"] = "stat after expiry is the real per-op round trip; the immediate repeat shows the actimeo=1 cache."
    return res


# ---------------------------------------------------------------- experiment 3
def exp3_listing(ctx, roots):
    dirs = ctx.state["dirs"] = {}  # path -> record (kept in memory only)
    res = {}
    ctx.budget(ctx.a.bfs_seconds)
    max_dirs = int(ctx.a.bfs_dirs * min(1.0, ctx.a.time_scale))
    conc = min(2, ctx.a.max_conc)

    def add(rec, depth, root_i):
        if rec["ok"]:
            rec.update({"depth": depth, "root": root_i})
            dirs[rec["path"]] = rec

    frontier = [(r, 0, i) for i, r in enumerate(roots)]
    errs = 0
    while frontier and len(dirs) < max_dirs and not ctx.expired():
        room = max_dirs - len(dirs)
        if len(frontier) > room:
            frontier = ctx.rng.sample(frontier, room)
        out, _ = pool_map(ctx, frontier, lambda x: (list_dir(ctx, x[0]), x[1], x[2]), conc)
        nxt = []
        for rec, depth, ri in out:
            if not rec["ok"]:
                errs += 1
                continue
            add(rec, depth, ri)
            for d in rec["dirs"]:
                nxt.append((os.path.join(rec["path"], d), depth + 1, ri))
        frontier = nxt
    res["bfs_dirs_listed"] = len(dirs)
    res["bfs_stopped_by"] = "budget" if frontier else "tree exhausted"
    res["bfs_errors"] = errs
    ctx.budget(ctx.a.descent_seconds)
    descents = int(ctx.a.descents * min(1.0, ctx.a.time_scale))
    done = 0
    while done < descents and not ctx.expired():
        done += 1
        ri = ctx.rng.randrange(len(roots))
        cur, depth = roots[ri], 0
        while not ctx.expired():
            rec = dirs.get(cur)
            if rec is None:
                rec = list_dir(ctx, cur)
                if not rec["ok"]:
                    break
                add(rec, depth, ri)
                rec = dirs[cur]
            if not rec["dirs"] or (rec["files"] and ctx.rng.random() < 0.15):
                break
            cur = os.path.join(cur, ctx.rng.choice(rec["dirs"]))
            depth += 1
    res["descents"] = done
    recs = list(dirs.values())
    res["dirs_total_sampled"] = len(recs)
    depth_hist = collections.Counter(r["depth"] for r in recs)
    res["depth_histogram"] = {str(k): depth_hist[k] for k in sorted(depth_hist)}
    res["max_depth_seen"] = max(depth_hist) if depth_hist else 0
    ents = [r["n"] for r in recs]
    res["entries_per_dir"] = {"n": len(ents), "p50": pct(ents, 50), "p90": pct(ents, 90), "p99": pct(ents, 99),
                              "max": max(ents) if ents else 0, "mean": round(sum(ents) / max(1, len(ents)), 1)}
    fb = [r for r in recs if r["files"]]
    res["file_bearing_dirs"] = len(fb)
    res["files_per_file_bearing_dir_mean"] = round(sum(len(r["files"]) for r in fb) / max(1, len(fb)), 1)
    res["subdirs_per_dir_mean"] = round(sum(len(r["dirs"]) for r in recs) / max(1, len(recs)), 2)
    res["leaf_dir_fraction"] = round(sum(1 for r in recs if not r["dirs"]) / max(1, len(recs)), 3)
    b = collections.defaultdict(lambda: {"dirs": 0, "entries": 0, "secs": 0.0, "lat": []})
    for r in recs:
        x = b[dir_bucket(r["n"])]
        x["dirs"] += 1
        x["entries"] += r["n"]
        x["secs"] += r["lat"]
        x["lat"].append(r["lat"])
    res["by_dir_size"] = {k: {"dirs": v["dirs"], "mean_entries": round(v["entries"] / v["dirs"], 1),
                              "entries_per_s_aggregate": round(v["entries"] / v["secs"], 1) if v["secs"] else None,
                              "list_latency_ms": lat_stats(v["lat"])} for k, v in sorted(b.items())}
    fit = linfit([r["n"] for r in recs], [r["lat"] for r in recs])
    res["latency_model"] = fit and {"fixed_ms_per_dir": round(fit["a_s"] * 1000, 3),
                                    "us_per_entry": round(fit["b_s_per_entry"] * 1e6, 2), "at_concurrency": conc}
    ec = collections.Counter()
    for r in recs:
        for n in r["files"]:
            ec[ext_of(n)] += 1
    res["extension_counts_in_sampled_dirs"] = dict(ec.most_common(30))
    res["files_seen"] = sum(ec.values())
    # concurrency scaling on a fixed stratified set of directories
    pick = []
    for k in ("small(<10)", "medium(10-200)", "large(>200)"):
        grp = [r["path"] for r in recs if dir_bucket(r["n"]) == k]
        ctx.rng.shuffle(grp)
        pick += grp[:100]
    ctx.rng.shuffle(pick)
    scale = {}
    for c in [1] + [x for x in ALLOWED_CONC if 1 < x <= ctx.a.max_conc] + [1]:
        if ctx.g.stop.is_set():
            break
        time.sleep(1.5)
        ctx.budget(ctx.a.scale_seconds)
        out, el = pool_map(ctx, pick, lambda p: list_dir(ctx, p), c)
        okr = [r for r in out if r["ok"]]
        key = "c%d" % c + ("_repeat" if "c%d" % c in scale else "")
        scale[key] = {"dirs": len(okr), "seconds": round(el, 2), "dirs_per_s": round(len(okr) / el, 1) if el else None,
                      "entries_per_s": round(sum(r["n"] for r in okr) / el, 1) if el else None,
                      "dir_latency_ms": lat_stats([r["lat"] for r in okr]), "errors": len(out) - len(okr),
                      "complete": len(out) == len(pick)}
    res["concurrency_scaling"] = scale
    return res


# ---------------------------------------------------------------- experiment 4
def build_file_sample(ctx, n_target, per_dir_cap=150):
    recs = [r for r in ctx.state["dirs"].values() if r["files"]]
    ctx.rng.shuffle(recs)
    out = []
    for r in recs:
        names = r["files"]
        if len(names) > per_dir_cap:
            names = ctx.rng.sample(names, per_dir_cap)
        out += [(r["path"], n) for n in names]
        if len(out) >= n_target * 3:
            break
    ctx.rng.shuffle(out)
    return out[:n_target]


def exp4_stat(ctx, roots):
    st = ctx.state
    res = {}
    n = int(ctx.a.stat_files * min(1.0, ctx.a.time_scale))
    sample = build_file_sample(ctx, n)
    res["sample_files"] = len(sample)
    res["sample_dirs"] = len({p for p, _ in sample})
    time.sleep(2.0)  # let the readdir-primed attribute cache expire (actimeo=1)
    rows = st["file_rows"] = []  # (dir_path, name, ext, size, mtime_ns), memory only
    levels = {}
    first_level = True
    for c in [x for x in ALLOWED_CONC if x <= ctx.a.max_conc]:
        if ctx.g.stop.is_set():
            break
        time.sleep(1.5)
        ctx.budget(ctx.a.stat_seconds)
        lats, errs = [], 0

        def work(item):
            ok, r, lat, err = ctx.g.timed("stat", os.stat, os.path.join(item[0], item[1]))
            return (item, ok, r, lat)
        out, el = pool_map(ctx, sample, work, c)
        for (p, nme), ok, r, lat in out:
            if ok:
                lats.append(lat)
                if first_level:
                    rows.append((p, nme, ext_of(nme), r.st_size, r.st_mtime_ns))
            else:
                errs += 1
        first_level = False
        levels["c%d" % c] = {"stats": len(lats), "seconds": round(el, 2), "files_per_s": round(len(lats) / el, 1) if el else None,
                             "latency_ms": lat_stats(lats), "errors": errs, "complete": len(out) == len(sample)}
    res["cold_stat_by_concurrency"] = levels
    dl = [r["path"] for r in st["dirs"].values() if 5 <= r["n"] <= 400]
    ctx.rng.shuffle(dl)
    dl = dl[:60]

    def pass_a(p):  # scandir, d_type only
        def go():
            k = 0
            with os.scandir(p) as it:
                for e in it:
                    e.is_dir(follow_symlinks=False)
                    k += 1
            return k
        return ctx.g.timed("readdir", go)

    def pass_b(p):  # scandir + DirEntry.stat() immediately
        def go():
            k = 0
            with os.scandir(p) as it:
                for e in it:
                    e.stat(follow_symlinks=False)
                    k += 1
            return k
        return ctx.g.timed("readdir", go)

    ctx.budget(ctx.a.stat_seconds)
    timings = {}
    for name, fn in (("A_scandir_dtype_only", pass_a), ("B_scandir_plus_entry_stat", pass_b), ("A_repeat", pass_a)):
        time.sleep(1.6)
        tot, ents, lats = 0.0, 0, []
        for p in dl:
            if ctx.expired():
                break
            ok, r, lat, _ = fn(p)
            if ok:
                tot += lat
                ents += r
                lats.append(lat)
        timings[name] = {"dirs": len(lats), "entries": ents, "total_s": round(tot, 3),
                         "us_per_entry": round(tot / max(1, ents) * 1e6, 2), "dir_latency_ms": lat_stats(lats)}
    time.sleep(1.6)
    paths = []
    for p in dl:
        rec = st["dirs"].get(p)
        if rec:
            paths += [os.path.join(p, n) for n in rec["files"][:50]]
    lats = []
    for x in paths:
        if ctx.expired():
            break
        ok, r, lat, _ = ctx.g.timed("stat", os.stat, x)
        if ok:
            lats.append(lat)
    timings["C_separate_cold_os_stat_per_file"] = {"files": len(lats), "us_per_entry": round(sum(lats) / max(1, len(lats)) * 1e6, 2),
                                                    "latency_ms": lat_stats(lats)}
    b, a_ = timings["B_scandir_plus_entry_stat"], timings["A_scandir_dtype_only"]
    extra = b["us_per_entry"] - a_["us_per_entry"]
    cstat = timings["C_separate_cold_os_stat_per_file"]["us_per_entry"]
    timings["interpretation"] = {
        "extra_us_per_entry_for_entry_stat": round(extra, 2),
        "cold_stat_us_per_file": cstat,
        "entry_stat_much_cheaper_than_cold_stat": bool(cstat and extra < 0.6 * cstat),
        "note": "If DirEntry.stat() right after the listing costs far less than a cold os.stat, the readdir reply "
                "already carried the attributes (cifs primes the inode cache) and no extra round trip is paid."}
    res["listing_styles"] = timings
    return res


# ---------------------------------------------------------------- experiment 5
def exp5_dirmtime(ctx, roots):
    st = ctx.state
    res = {}
    cand = [r for r in st["dirs"].values() if 1 <= r["n"] <= 400]
    ctx.rng.shuffle(cand)
    cand = cand[:ctx.a.mtime_dirs]
    ctx.budget(ctx.a.mtime_seconds)
    time.sleep(1.5)

    def work(rec):
        p = rec["path"]
        ok, s0, _, _ = ctx.g.timed("stat", os.stat, p)
        if not ok:
            return None
        kids = []

        def go():
            with os.scandir(p) as it:
                for e in it:
                    try:
                        s = e.stat(follow_symlinks=False)
                        kids.append((e.is_dir(follow_symlinks=False), s.st_mtime_ns))
                    except OSError:
                        pass
            return len(kids)
        ok2, _, _, _ = ctx.g.timed("readdir", go)
        if not ok2 or not kids:
            return None
        ok3, s1, _, _ = ctx.g.timed("stat", os.stat, p)
        return (s0.st_mtime_ns, s1.st_mtime_ns if ok3 else None, kids)
    out, el = pool_map(ctx, cand, work, min(4, ctx.a.max_conc))
    rows = [o for o in out if o]
    ge = eq = lt = changed = 0
    gaps_days = []
    allm = []
    for d0, d1, kids in rows:
        mx = max(m for _, m in kids)
        ge += d0 >= mx
        eq += d0 == mx
        lt += d0 < mx
        changed += (d1 is not None and d1 != d0)
        gaps_days.append((d0 - mx) / 86400e9)
        allm += [m for _, m in kids] + [d0]
    n = len(rows)
    res["dirs_compared"] = n
    res["children_stat"] = sum(len(k) for _, _, k in rows)
    res["frac_dir_mtime_ge_max_child_mtime"] = round(ge / n, 4) if n else None
    res["frac_dir_mtime_eq_max_child_mtime"] = round(eq / n, 4) if n else None
    res["frac_dir_mtime_lt_max_child_mtime"] = round(lt / n, 4) if n else None
    res["dirs_whose_mtime_changed_during_read"] = changed
    res["dir_minus_max_child_days"] = {"p5": round(pct(gaps_days, 5), 2), "p50": round(pct(gaps_days, 50), 2),
                                       "p95": round(pct(gaps_days, 95), 2)} if gaps_days else None
    allm += [r[4] for r in st.get("file_rows", [])]
    tot = len(allm)
    if tot:
        def frac(f):
            return round(sum(1 for m in allm if f(m)) / tot, 4)
        res["mtime_granularity"] = {
            "samples": tot,
            "whole_seconds": frac(lambda m: m % 10**9 == 0),
            "whole_and_even_seconds": frac(lambda m: m % (2 * 10**9) == 0),
            "odd_seconds": frac(lambda m: (m // 10**9) % 2 == 1),
            "multiple_of_1ms": frac(lambda m: m % 10**6 == 0),
            "multiple_of_1us": frac(lambda m: m % 10**3 == 0),
            "multiple_of_100ns": frac(lambda m: m % 100 == 0),
            "has_subsecond_part": frac(lambda m: m % 10**9 != 0),
        }
    files = [os.path.join(p, nme) for p, nme, *_ in st.get("file_rows", [])[:200]]
    ac = {"files": len(files)}
    if files:
        time.sleep(1.5)
        a1, a2, a3 = [], [], []
        same = tot_ = 0
        for f in files:
            if ctx.expired():
                break
            ok1, s1, l1, _ = ctx.g.timed("stat", os.stat, f)
            ok2, s2, l2, _ = ctx.g.timed("stat_hot", os.stat, f)
            if ok1 and ok2:
                a1.append(l1)
                a2.append(l2)
                tot_ += 1
                same += (s1.st_mtime_ns == s2.st_mtime_ns and s1.st_size == s2.st_size)
        time.sleep(2.5)
        for f in files[:tot_]:
            ok3, s3, l3, _ = ctx.g.timed("stat", os.stat, f)
            if ok3:
                a3.append(l3)
        ac.update({"first_stat_ms": lat_stats(a1), "immediate_repeat_ms": lat_stats(a2), "after_2p5s_ms": lat_stats(a3),
                   "values_identical_immediate": round(same / max(1, tot_), 4)})
    res["attribute_cache"] = ac
    res["proves"] = ("Only a read-only snapshot property: whether the directory mtime is at least as new as every direct child's "
                     "mtime. dir_mtime >= max_child_mtime is necessary for 'directory mtime tracks child changes' but not sufficient.")
    res["does_not_prove"] = ("That adding, deleting or renaming an entry updates the directory mtime on this stack, that "
                             "actimeo=1/server caching propagates it promptly, or that mtime-preserving bulk copies behave the same. "
                             "Those need a write; see the manual procedure in the report.")
    return res


# ---------------------------------------------------------------- experiment 6
def header_read(ctx, path, n):
    """One open + one pread of n bytes (looped only on a short read before EOF)."""
    def go():
        t0 = time.perf_counter()
        try:
            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOATIME", 0))
        except PermissionError:
            fd = os.open(path, os.O_RDONLY)
        t1 = time.perf_counter()
        try:
            buf = os.pread(fd, n, 0)
            calls = 1
            while len(buf) < n:
                more = os.pread(fd, n - len(buf), len(buf))
                calls += 1
                if not more:
                    break
                buf += more
            t2 = time.perf_counter()
            try:
                os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
            except (OSError, AttributeError):
                pass
        finally:
            os.close(fd)
        return buf, t1 - t0, t2 - t1, calls
    ok, r, lat, err = ctx.g.timed("read%d" % n, go, nbytes=lambda r: len(r[0]))
    if not ok:
        return None
    return {"buf": r[0], "open_s": r[1], "read_s": r[2], "total_s": lat, "preads": r[3]}


def exp6_headers(ctx, roots):
    st = ctx.state
    res = {}
    rows = st.get("file_rows", [])
    byext = collections.defaultdict(list)
    for p, nme, e, size, mt in rows:
        byext[e].append(size)
    res["size_by_extension"] = {e: size_stats(v) for e, v in sorted(byext.items(), key=lambda kv: -len(kv[1])) if len(v) >= 3}
    media = [(p, nme, e, s) for p, nme, e, s, _ in rows if e in READ_EXTS]
    res["media_files_in_sample"] = len(media)
    res["media_size_overall"] = size_stats([m[3] for m in media])
    res["media_fraction_of_sample_files"] = round(len(media) / max(1, len(rows)), 3)
    ext_counts = collections.Counter(e for _, _, e, _ in media)
    by = collections.defaultdict(list)
    for m in media:
        by[m[2]].append(m)
    for v in by.values():
        ctx.rng.shuffle(v)
    pool = []
    exts = [e for e, _ in ext_counts.most_common()]
    idx = {e: 0 for e in exts}
    while exts:
        for e in list(exts):
            if idx[e] >= len(by[e]):
                exts.remove(e)
                continue
            pool.append(by[e][idx[e]])
            idx[e] += 1
    res["pool_size"] = len(pool)
    if not pool:
        res["error"] = "no media files in sample"
        return res
    ctx.state["header_pool"] = pool
    cursor = itertools.count()
    cells = {}
    reused = False
    ext_lat = collections.defaultdict(lambda: collections.defaultdict(list))
    for sz in (65536, 262144, 1048576):
        for c in [x for x in ALLOWED_CONC if x <= ctx.a.read_max_conc]:
            if ctx.g.stop.is_set():
                break
            time.sleep(1.0)
            ctx.budget(ctx.a.cell_seconds)
            items = []
            for _ in range(ctx.a.header_files_per_cell):
                i = next(cursor)
                reused |= i >= len(pool)
                items.append(pool[i % len(pool)])

            def work(m):
                return (m[2], m[3], header_read(ctx, os.path.join(m[0], m[1]), sz))
            out, el = pool_map(ctx, items, work, c)
            good = [(e, fs, r) for e, fs, r in out if r]
            tot = [r["total_s"] for _, _, r in good]
            nb = sum(len(r["buf"]) for _, _, r in good)
            cells["%dk_c%d" % (sz // 1024, c)] = {
                "files": len(good), "errors": len(out) - len(good), "seconds": round(el, 2),
                "files_per_s": round(len(good) / el, 2) if el else None,
                "MB_per_s": round(nb / MB / el, 2) if el else None, "bytes": nb,
                "total_ms": lat_stats(tot), "open_ms": lat_stats([r["open_s"] for _, _, r in good]),
                "read_ms": lat_stats([r["read_s"] for _, _, r in good]),
                "mean_preads": round(sum(r["preads"] for _, _, r in good) / max(1, len(good)), 2),
                "complete": len(out) == len(items)}
            for e, fs, r in good:
                ext_lat[e][sz].append(r["total_s"])
            if c == 1 and tot:
                ctx.g.set_baseline("read%d" % sz, pct(tot, 95))
    res["cells"] = cells
    res["files_reused_across_cells"] = reused
    res["per_extension_header_latency_ms_p50"] = {
        e: {"%dk" % (sz // 1024): round(pct(v, 50) * 1000, 2) for sz, v in d.items()} for e, d in ext_lat.items()}
    return res


def exp6b_prefix(ctx, roots):
    """Local exiftool analysis of how much of each format is needed (EXIF and embedded preview)."""
    exiftool = shutil.which("exiftool")
    if not exiftool:
        return {"skipped": "exiftool not installed"}
    rows = ctx.state.get("file_rows", [])
    by = collections.defaultdict(list)
    for p, nme, e, s, mt in rows:
        if e in READ_EXTS:
            by[e].append((p, nme, e, s))
    exts = sorted(by, key=lambda e: -len(by[e]))[:ctx.a.prefix_max_exts]
    tmp = tempfile.mkdtemp(prefix="nasbench-", dir=ctx.a.tmp_dir)
    ctx.budget(ctx.a.prefix_seconds)
    out_res = {}
    try:
        for e in exts:
            if ctx.expired():
                break
            files = by[e][:]
            ctx.rng.shuffle(files)
            files = files[:ctx.a.prefix_files_per_ext]
            outs, _ = pool_map(ctx, files, lambda m: (m, header_read(ctx, os.path.join(m[0], m[1]), PREFIX_LADDER[-1])),
                               min(2, ctx.a.read_max_conc))
            got = [(m, r["buf"], r["total_s"]) for m, r in outs if r]
            if not got:
                continue
            d = os.path.join(tmp, e)
            for i, (m, buf, _) in enumerate(got):
                for pf in PREFIX_LADDER:
                    sub = os.path.join(d, str(pf))
                    os.makedirs(sub, exist_ok=True)
                    with open(os.path.join(sub, "f%d.%s" % (i, e)), "wb") as f:
                        f.write(buf[:pf])
            per_prefix = {}
            for pf in PREFIX_LADDER:
                sub = os.path.join(d, str(pf))
                cmd = ["nice", "-n", "10", exiftool, "-m", "-json", "-b", "-q", "-Make", "-Model", "-DateTimeOriginal", "-CreateDate",
                       "-PreviewImage", "-JpgFromRaw", "-OtherImage", "-ThumbnailImage", "-PreviewImageStart", "-PreviewImageLength",
                       "-JpgFromRawStart", "-JpgFromRawLength", "-OtherImageStart", "-OtherImageLength", sub]
                try:
                    pr = subprocess.run(cmd, capture_output=True, timeout=120)
                    js = json.loads(pr.stdout.decode("utf-8", "replace") or "[]")
                except Exception:
                    js = []
                per_prefix[pf] = {os.path.basename(j.get("SourceFile", "")): j for j in js}
            exif_need, prev_need, prev_end, prev_len, fsz, whichtag = [], [], [], [], [], []
            for i, (m, buf, _) in enumerate(got):
                fn = "f%d.%s" % (i, e)
                en = pn = None
                fsz.append(m[3])
                for pf in PREFIX_LADDER:
                    j = per_prefix.get(pf, {}).get(fn, {})
                    if en is None and j.get("Model") and (j.get("DateTimeOriginal") or j.get("CreateDate")):
                        en = min(pf, m[3])
                    if pn is None:
                        for tag in ("PreviewImage", "JpgFromRaw", "OtherImage", "ThumbnailImage"):
                            v = j.get(tag)
                            if isinstance(v, str) and v.startswith("base64:"):
                                try:
                                    bb = base64.b64decode(v[7:])
                                except Exception:
                                    continue
                                if len(bb) >= 4096 and bb[:2] == b"\xff\xd8" and bb[-2:] == b"\xff\xd9":
                                    pn = min(pf, m[3])
                                    prev_len.append(len(bb))
                                    whichtag.append(tag)
                                    break
                    if en is not None and pn is not None:
                        break
                exif_need.append(en)
                prev_need.append(pn)
                jf = per_prefix.get(PREFIX_LADDER[-1], {}).get(fn, {})
                for t in ("PreviewImage", "JpgFromRaw", "OtherImage"):
                    s, l = jf.get(t + "Start"), jf.get(t + "Length")
                    if isinstance(s, int) and isinstance(l, int) and l > 4096:
                        prev_end.append(s + l)
                        break

            def dist(v):
                ok = [x for x in v if x is not None]
                return {"files": len(v), "found_in_2MB": len(ok), "p50": pct(ok, 50), "p95": pct(ok, 95), "max": max(ok) if ok else None}
            out_res[e] = {"sampled": len(got), "file_size_median": int(pct(fsz, 50)),
                          "exif_prefix_bytes_needed": dist(exif_need),
                          "preview_prefix_bytes_needed_ladder": dist(prev_need),
                          "preview_end_offset_bytes_from_ifd": {"files": len(prev_end), "p50": pct(prev_end, 50), "p95": pct(prev_end, 95),
                                                                "max": max(prev_end) if prev_end else None},
                          "preview_jpeg_len_median": int(pct(prev_len, 50)) if prev_len else None,
                          "preview_source_tags": dict(collections.Counter(whichtag)),
                          "read_2MB_ms_p50": round(pct([t for _, _, t in got], 50) * 1000, 2)}
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return {"by_extension": out_res, "ladder_bytes": PREFIX_LADDER,
            "method": ("Read the first 2 MB of each sampled file once, copy prefixes locally, run exiftool on each prefix; 'needed' is "
                       "the smallest ladder prefix with Model + a capture time (EXIF) or a complete JPEG preview (FFD8..FFD9, >=4 KB). "
                       "'preview_end_offset' is start+length from the file's own IFD tags. Local temp copies are deleted.")}


# ---------------------------------------------------------------- experiment 7
def read_chunks(ctx, path, offsets, chunk):
    def go():
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOATIME", 0))
        n = 0
        try:
            for off in offsets:
                b = os.pread(fd, chunk, off)
                n += len(b)
                if len(b) < chunk:
                    break
            try:
                os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
            except (OSError, AttributeError):
                pass
        finally:
            os.close(fd)
        return n
    return ctx.g.timed("bulk", go, nbytes=lambda r: r)


def exp7_throughput(ctx, roots):
    rows = ctx.state.get("file_rows", [])
    big = [r for r in rows if r[3] >= 32 * MB]
    res = {"candidate_files_ge_32MB": len(big)}
    if len(big) < 2:
        big = sorted(rows, key=lambda r: -r[3])[:8]
        res["note"] = "few large files; used the largest available, tests truncated by file size"
    if not big:
        return {"error": "no files"}
    ctx.rng.shuffle(big)
    cur = itertools.cycle(big)
    ctx.budget(ctx.a.bulk_seconds)
    left = [ctx.a.bulk_bytes]
    tests = {}

    def run(name, chunk, nchunks, conc, random_off=False):
        if ctx.expired() or left[0] <= 0 or conc > ctx.a.read_max_conc:
            return
        work = []
        for _ in range(conc):
            f = next(cur)
            n = max(1, min(nchunks, f[3] // chunk))
            if random_off:
                offs = sorted(ctx.rng.randrange(0, max(1, (f[3] - chunk) // chunk)) * chunk for _ in range(n))
            else:
                offs = [i * chunk for i in range(n)]
            work.append((os.path.join(f[0], f[1]), offs))
        if sum(len(o) * chunk for _, o in work) > left[0]:
            return
        time.sleep(0.5)
        out, el = pool_map(ctx, work, lambda w: read_chunks(ctx, w[0], w[1], chunk), conc)
        nb = sum(r[1] for r in out if r[0])
        left[0] -= nb
        tests[name] = {"bytes": nb, "seconds": round(el, 3), "MB_per_s": round(nb / MB / el, 1) if el else None,
                       "chunk": chunk, "concurrency": conc, "errors": sum(1 for r in out if not r[0])}

    run("seq_8MB_chunks_c1", 8 * MB, 8, 1)
    run("seq_4MB_chunks_c1", 4 * MB, 8, 1)
    run("seq_1MB_chunks_c1", 1 * MB, 16, 1)
    run("random_8MB_chunks_c1", 8 * MB, 12, 1, True)
    run("seq_8MB_c2", 8 * MB, 3, 2)
    run("seq_8MB_c4", 8 * MB, 3, 4)
    res["tests"] = tests
    res["bytes_total"] = ctx.a.bulk_bytes - left[0]
    res["note"] = "With rsize=4 MB an 8 MB pread is two 4 MB SMB reads; compare the 8/4/1 MB chunk results."
    return res


# ---------------------------------------------------------------- experiment 8 (no I/O)
def best_conc(by_c, tol=0.9):
    """Smallest concurrency reaching tol of the best throughput. by_c: {conc: rate}."""
    vals = {c: v for c, v in by_c.items() if v}
    if not vals:
        return None
    best = max(vals.values())
    return min(c for c in sorted(vals) if vals[c] >= tol * best)


def exp8_estimate(res, a):
    e3, e4, e5, e6 = (res.get(k) or {} for k in ("3_listing", "4_stat", "5_dirmtime", "6_headers"))
    e6b = res.get("6b_prefix") or {}
    if not e3 or not e6 or not (e6.get("media_size_overall") or {}).get("mean"):
        return {"skipped": "needs experiments 3, 4 and 6"}
    mean_size = e6["media_size_overall"]["mean"]
    media_frac = e6.get("media_fraction_of_sample_files") or 0.9
    model = e3.get("latency_model") or {}
    a_s = (model.get("fixed_ms_per_dir") or 0) / 1000.0
    b_s = (model.get("us_per_entry") or 0) / 1e6
    fpd = max(1.0, e3.get("files_per_file_bearing_dir_mean") or 50)
    scale = e3.get("concurrency_scaling") or {}
    s1 = (scale.get("c1") or {}).get("dirs_per_s")
    speed = {c: scale["c%d" % c]["dirs_per_s"] / s1 for c in ALLOWED_CONC
             if s1 and (scale.get("c%d" % c) or {}).get("dirs_per_s")}
    cs = e4.get("cold_stat_by_concurrency") or {}
    stat_rate = {int(k[1:]): v["files_per_s"] for k, v in cs.items() if v.get("files_per_s")}
    cells = e6.get("cells") or {}
    est = {"assumptions": {"mean_media_file_bytes": mean_size, "media_fraction_of_files": media_frac,
                           "files_per_file_bearing_dir": fpd, "dir_list_fixed_ms": round(a_s * 1000, 3),
                           "dir_list_us_per_entry": round(b_s * 1e6, 2),
                           "listing_speedup_vs_c1": {str(k): round(v, 2) for k, v in speed.items()},
                           "library_TB_low_mid_high": [a.library_tb_low, a.library_tb_mid, a.library_tb_high]},
           "method": {
               "file_count": "N_media = library_bytes / mean media file size (stat sample); N_total = N_media / media fraction.",
               "dir_count": "D = N_total / mean files per file-bearing dir x 1.15 container dirs. BFS biases toward shallow dirs: treat as +-2x.",
               "discovery": "T(c) = (D*a + N*b) / listing speedup(c); a, b from the listing regression, speedup measured on 300 fixed dirs.",
               "cold_stat": "added N / cold stat files/s at c when sizes and mtimes are needed.",
               "metadata": "N_media / measured header-read files/s at c (already latency and bandwidth bound).",
               "rescan_with_skip": "each known dir stat'd once (stored subdir list); changed fraction re-listed and re-stat'd.",
               "thumbnails": "N_media x (fixed + bytes/bw) from the c1 header cells, scaled by the 256K concurrency speedup."}}
    rows = []
    for tb in (a.library_tb_low, a.library_tb_mid, a.library_tb_high):
        nm = tb * 1e12 / mean_size
        nt = nm / media_frac
        D = nt / fpd * 1.15
        row = {"library_TB": tb, "media_files": int(nm), "total_files": int(nt), "dirs": int(D)}
        disc, disc_stat, skip, meta, thumb = {}, {}, {}, {}, {}
        for c in ALLOWED_CONC:
            if c > a.max_conc:
                continue
            sp, sr = speed.get(c), stat_rate.get(c)
            if sp:
                t_list = (D * a_s + nt * b_s) / sp
                disc[str(c)] = round(t_list / 60, 1)
                if sr:
                    full = t_list + nt / sr
                    disc_stat[str(c)] = round(full / 60, 1)
                    skip[str(c)] = {"1pct_dirs_changed_min": round((D / sr + 0.01 * full) / 60, 2),
                                    "5pct_dirs_changed_min": round((D / sr + 0.05 * full) / 60, 2)}
            for sz in (65536, 262144, 1048576):
                cell = cells.get("%dk_c%d" % (sz // 1024, c)) or {}
                if cell.get("files_per_s"):
                    meta.setdefault("%dk" % (sz // 1024), {})[str(c)] = round(nm / cell["files_per_s"] / 3600, 2)
        row["discovery_list_only_min_by_conc"] = disc
        row["discovery_list_plus_cold_stat_min_by_conc"] = disc_stat
        row["rescan_without_dir_skip_min_by_conc"] = disc_stat
        row["rescan_with_dir_skip_by_conc"] = skip
        row["metadata_hours_by_header_size_and_conc"] = meta
        pe = []
        for v in (e6b.get("by_extension") or {}).values():
            x = (v.get("preview_end_offset_bytes_from_ifd") or {}).get("p50") or (v.get("preview_prefix_bytes_needed_ladder") or {}).get("p50")
            if x:
                pe.append(x)
        xs = [sz for sz in (65536, 262144, 1048576) if (cells.get("%dk_c1" % (sz // 1024)) or {}).get("total_ms", {}).get("p50")]
        ys = [cells["%dk_c1" % (sz // 1024)]["total_ms"]["p50"] / 1000.0 for sz in xs]
        fit = linfit(xs, ys)
        if pe and fit:
            B = sum(pe) / len(pe)
            c1 = (cells.get("256k_c1") or {}).get("files_per_s")
            for c in ALLOWED_CONC:
                cc = (cells.get("256k_c%d" % c) or {}).get("files_per_s")
                if c <= a.max_conc and c1 and cc:
                    thumb[str(c)] = round(nm * (fit["a_s"] + fit["b_s_per_entry"] * B) / (cc / c1) / 3600, 2)
            row["thumbnail_preview_bytes_assumed"] = int(B)
            row["thumbnail_extract_hours_by_conc"] = thumb
        rows.append(row)
    est["by_library_size"] = rows
    est["best_concurrency_90pct_of_max"] = {
        "listing": best_conc({c: (scale.get("c%d" % c) or {}).get("dirs_per_s") for c in ALLOWED_CONC}),
        "cold_stat": best_conc(stat_rate),
        **{"header_%dk" % (sz // 1024): best_conc({c: (cells.get("%dk_c%d" % (sz // 1024, c)) or {}).get("files_per_s")
                                                  for c in ALLOWED_CONC}) for sz in (65536, 262144, 1048576)}}
    est["dir_mtime_snapshot_evidence"] = {"frac_ge": e5.get("frac_dir_mtime_ge_max_child_mtime"),
                                           "frac_eq": e5.get("frac_dir_mtime_eq_max_child_mtime")}
    return est


# ---------------------------------------------------------------- local synthetic tree (baseline)
def make_local_tree(base, files=400, seed=7):
    """Generate a few hundred small files under base (a local directory), nested, with mixed extensions."""
    rng = random.Random(seed)
    jpeg = base64.b64decode(
        "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/"
        "wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACv/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q==")
    exts = ["jpg"] * 5 + ["dng", "cr2", "arw", "nef", "png", "heic"]
    os.makedirs(base, exist_ok=True)
    made = 0
    for i in range(20):
        d = os.path.join(base, "d%02d" % i, "s%d" % (i % 3))
        os.makedirs(d, exist_ok=True)
        for j in range(files // 20):
            e = rng.choice(exts)
            size = max(len(jpeg) + 10, min(int(rng.lognormvariate(11.0, 1.0)), 3 * MB))
            with open(os.path.join(d, "f%03d.%s" % (j, e)), "wb") as f:
                f.write(jpeg + os.urandom(size - len(jpeg)))
            made += 1
    return made


# ---------------------------------------------------------------- plan / main
EXPERIMENTS = [
    ("1_mount", "Mount facts, first and later access", exp1_mount),
    ("2_latency", "stat latency baseline and one listing round trip", exp2_latency),
    ("3_listing", "Bounded BFS + random descents; listing throughput by directory size; concurrency scaling", exp3_listing),
    ("4_stat", "Cold stat throughput at 1/2/4/8 threads; scandir styles compared", exp4_stat),
    ("5_dirmtime", "Directory mtime vs children (read-only), timestamp granularity, attribute cache", exp5_dirmtime),
    ("6_headers", "Header reads 64K/256K/1M at 1/2/4/8; per-extension size distribution", exp6_headers),
    ("6b_prefix", "Local exiftool analysis of the prefix needed for EXIF and embedded preview", exp6b_prefix),
    ("7_throughput", "Sequential/random 8 MB chunk reads from large files", exp7_throughput),
]


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--root", action="append", default=[], help="folder to measure (repeatable)")
    p.add_argument("--roots-from-settings", metavar="SETTINGS_JSON",
                   help="read roots from an Immage settings.json (key immage.library); keeps those under --mount-prefix")
    p.add_argument("--mount-prefix", default=None)
    p.add_argument("--label", default="run", help="label stored in the results (e.g. nas, local)")
    p.add_argument("--out", default="results.json")
    p.add_argument("--only", default="", help="comma list of experiment numbers, e.g. 1,2,3 (3 and 4 feed 5-8)")
    p.add_argument("--dry-run", action="store_true", help="print the plan and limits; touch nothing")
    p.add_argument("--generate-local", metavar="DIR", help="create a synthetic local tree of a few hundred small files, then measure it")
    p.add_argument("--max-conc", type=int, default=8, choices=ALLOWED_CONC)
    p.add_argument("--read-max-conc", type=int, default=None, choices=ALLOWED_CONC,
                   help="separate cap for experiments that read file bytes (6, 6b, 7); default = --max-conc. "
                        "Spinning-disk NAS units saturate on small reads at 2 and the latency stop trips above that.")
    p.add_argument("--max-ops-per-sec", type=float, default=2500.0, help="global rate limit on guarded operations (0 = off)")
    p.add_argument("--max-bytes", type=int, default=2 * 1024**3, help="cap on total bytes read (default 2 GB)")
    p.add_argument("--max-total-seconds", type=int, default=45 * 60)
    p.add_argument("--latency-factor", type=float, default=5.0, help="emergency stop if window p95 exceeds factor x baseline p95")
    p.add_argument("--latency-floor-ms", type=float, default=30.0, help="never stop below this absolute p95")
    p.add_argument("--max-error-rate", type=float, default=0.02)
    p.add_argument("--time-scale", type=float, default=1.0, help="scale all experiment budgets (0.2 = quick smoke run)")
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--library-tb-low", type=float, default=1.0)
    p.add_argument("--library-tb-mid", type=float, default=1.5)
    p.add_argument("--library-tb-high", type=float, default=2.0)
    p.add_argument("--tmp-dir", default=None, help="LOCAL scratch dir for exiftool prefix copies (never on the measured mount)")
    p.add_argument("--bfs-seconds", type=int, default=120)
    p.add_argument("--bfs-dirs", type=int, default=1500)
    p.add_argument("--descent-seconds", type=int, default=90)
    p.add_argument("--descents", type=int, default=150)
    p.add_argument("--scale-seconds", type=int, default=40)
    p.add_argument("--stat-files", type=int, default=20000)
    p.add_argument("--stat-seconds", type=int, default=60)
    p.add_argument("--mtime-dirs", type=int, default=250)
    p.add_argument("--mtime-seconds", type=int, default=120)
    p.add_argument("--header-files-per-cell", type=int, default=40)
    p.add_argument("--cell-seconds", type=int, default=45)
    p.add_argument("--prefix-files-per-ext", type=int, default=8)
    p.add_argument("--prefix-max-exts", type=int, default=12)
    p.add_argument("--prefix-seconds", type=int, default=240)
    p.add_argument("--bulk-seconds", type=int, default=120)
    p.add_argument("--bulk-bytes", type=int, default=400 * MB)
    return p.parse_args(argv)


def load_roots(a):
    roots = list(a.root)
    if a.roots_from_settings:
        with open(os.path.expanduser(a.roots_from_settings)) as f:
            s = json.load(f)
        v = s.get("immage.library")
        v = json.loads(v) if isinstance(v, str) else v
        pre = (a.mount_prefix or "").rstrip("/") + "/"
        roots += [x for x in (v or {}).get("folders", []) if a.mount_prefix and (x + "/").startswith(pre)]
    roots = sorted(set(os.path.abspath(r) for r in roots), key=len)
    out = []
    for r in roots:
        if not any(r.startswith(o.rstrip("/") + "/") for o in out):
            out.append(r)
    return out


def write_out(a, results, guard, t_start):
    results["guard"] = {"ops": guard.ops, "errors": guard.errs, "error_codes": dict(guard.errcodes), "bytes_read": guard.bytes,
                        "emergency_stop": guard.reason, "wall_s": round(time.monotonic() - t_start, 1)}
    tmp = a.out + ".tmp"
    with open(tmp, "w") as f:
        json.dump(results, f, indent=1, default=str)
    os.replace(tmp, a.out)


def main(argv=None):
    a = parse_args(argv or sys.argv[1:])
    if a.tmp_dir is None:
        a.tmp_dir = tempfile.gettempdir()
    if a.read_max_conc is None:
        a.read_max_conc = a.max_conc
    a.read_max_conc = min(a.read_max_conc, a.max_conc)
    only = {x.strip() for x in a.only.split(",") if x.strip()}
    sel = [(i, d, f) for i, d, f in EXPERIMENTS if not only or i.split("_")[0] in only or i in only]
    if a.generate_local:
        if a.dry_run:
            print("would generate a synthetic local tree (a few hundred small files) under the given directory")
        else:
            log("generated %d local files" % make_local_tree(a.generate_local))
            a.root.append(a.generate_local)
    roots = load_roots(a)
    if a.dry_run:
        print("DRY RUN: nothing is accessed.")
        print("roots: %d (names are never printed)" % len(roots))
        print("limits: concurrency <= %d (of 1,2,4,8), %.0f ops/s global, %d MB bytes read, %d s total, latency stop %.0fx baseline "
              "p95 (floor %.0f ms), error stop %.0f%%, stop when the mount disappears"
              % (a.max_conc, a.max_ops_per_sec, a.max_bytes // MB, a.max_total_seconds, a.latency_factor, a.latency_floor_ms,
                 a.max_error_rate * 100))
        print("access: read-only syscalls only (stat, scandir, open O_RDONLY|O_NOATIME, pread, fadvise); local scratch under %s" % a.tmp_dir)
        for i, d, _ in sel:
            print("  experiment %-13s %s" % (i, d))
        print("  8_estimate            Extrapolation from the measured rates (no I/O)")
        return 0
    if not roots:
        print("no roots given (use --root, or --roots-from-settings with --mount-prefix)", file=sys.stderr)
        return 2
    try:
        os.nice(10)
    except OSError:
        pass
    try:
        subprocess.run(["ionice", "-c3", "-p", str(os.getpid())], capture_output=True, timeout=5)
    except Exception:
        pass
    mp, ents = mounts_for(roots[0])
    fst = ents[-1]["fstype"] if ents else None
    network = fst in ("cifs", "smb3", "nfs", "nfs4", "fuse.sshfs")
    guard = Guard(a, mp if network else "", fst)
    ctx = Ctx(a, guard, {})
    results = {"label": a.label, "started": time.strftime("%Y-%m-%dT%H:%M:%S"), "tool_version": 1, "python": sys.version.split()[0],
               "limits": {"max_conc": a.max_conc, "max_ops_per_sec": a.max_ops_per_sec, "max_bytes": a.max_bytes,
                          "latency_factor": a.latency_factor, "max_error_rate": a.max_error_rate},
               "folder_labels": {"R%d" % (i + 1): ctx.hid(r) for i, r in enumerate(roots)}, "experiments": {}}
    t_start = time.monotonic()
    for i, d, fn in sel:
        if guard.stop.is_set():
            break
        if time.monotonic() - t_start > a.max_total_seconds:
            results["stopped_early"] = "total time budget reached before %s" % i
            break
        log("== %s: %s" % (i, d))
        t0 = time.monotonic()
        try:
            r = fn(ctx, roots)
        except StopRun:
            r = {"aborted": "emergency stop"}
        except Exception as ex:
            r = {"error": type(ex).__name__}
        r["_wall_s"] = round(time.monotonic() - t0, 1)
        r["_bytes_read_so_far"] = guard.bytes
        results["experiments"][i] = r
        log("   done in %.1fs, bytes read %.1f MB, ops %d, errors %d" % (r["_wall_s"], guard.bytes / MB, guard.ops, guard.errs))
        write_out(a, results, guard, t_start)
    results["8_estimate"] = exp8_estimate(results["experiments"], a)
    write_out(a, results, guard, t_start)
    log("results written; emergency stop: %s" % guard.reason)
    return 3 if guard.stop.is_set() else 0


if __name__ == "__main__":
    sys.exit(main())
