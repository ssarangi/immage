// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Photos tab of a person's page: the app's own Filmstrip in grid mode (virtualised, the same cells, ratings and
// thumbnail loader), fed with the photos faces_person_photos lists, a page at a time as the end scrolls into view.
// The photos are known by path alone, so their cells are the read-only "known" cells (a thumbnail from the disk
// cache, or none); opening one hands its path to the app.
import { useEffect, useMemo, useRef, useState } from "react";
import { Filmstrip } from "./Filmstrip";
import { ThumbLoader, type Known } from "./library";
import { t } from "./i18n";
import { peopleApi } from "./peopleStore";

const PAGE = 400;
const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;
const known = (path: string): Known => ({
  path, filename: baseName(path), rating: 0, rejected: false, labels: 0, edited: false, width: 0, height: 0, version: 0,
  exif: { maker: "", model: "", lens: "", exposure: 0, aperture: 0, iso: 0, focal_length: 0, exposure_bias: 0, focus_distance: 0, datetime: "" },
} as unknown as Known);

export function PersonPhotos({ person, onOpen, size = 180 }: { person: number; onOpen: (path: string) => void; size?: number }) {
  const [paths, setPaths] = useState<string[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState("");
  const asking = useRef(false);
  const loader = useMemo(() => new ThumbLoader(() => undefined, () => 128 * 1024 * 1024), []);
  const partners = useMemo(() => new Map(), []);
  const ask = (offset: number) => {
    if (asking.current) return;
    asking.current = true;
    peopleApi().personPhotos(person, offset, PAGE).then((r) => {
      setPaths((p) => (offset === 0 ? r.paths : [...p, ...r.paths.filter((x) => !p.includes(x))]));
      setTotal(r.total);
    }, (e) => setError(String(e))).finally(() => { asking.current = false; });
  };
  useEffect(() => { setPaths([]); setTotal(null); setError(""); asking.current = false; ask(0); /* eslint-disable-next-line react-hooks/exhaustive-deps */ }, [person]);
  const entries = useMemo(() => paths.map(known), [paths]);
  return (
    <div className="person-photos" aria-label={t("people.page.photos")}>
      {error ? <p className="people-state-line">{error}</p> : (
        <Filmstrip images={entries} partners={partners} pending={[]} empty={total === null ? t("people.state.loading") : t("people.page.noPhotos")} current={null}
                   loader={loader} height={150} grid gridSize={size} selection={new Set<number>()} candidate={null}
                   onPick={() => {}} onDouble={() => {}} onOpenKnown={(k) => onOpen(k.path)} onMenu={() => {}} onMeta={() => {}} onBackMenu={() => {}}
                   more={total !== null && paths.length < total ? () => ask(paths.length) : undefined} />
      )}
    </div>
  );
}
