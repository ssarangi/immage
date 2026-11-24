// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Find's People row editor (notes/faces-ux-spec.md 4.1): pick people by typing, keep them as include chips, flip a
// chip to exclude, choose Any of them or All together, and the two special cases: photos with no people and photos
// with faces still to name. The filter is data (facesFilter.ts); the photos it keeps are decided from path sets.
import { Ban, Plus, X } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { NO_FACES, normalizeFaces, setsUnavailable, type FaceFilter } from "./facesFilter";
import type { FindState } from "./findModel";
import { t } from "./i18n";
import { useFaceSetsVersion } from "./useFaceSets";
import { ensurePeople, usePeople } from "./peopleStore";

export function FacesEditor({ state, send }: { state: FindState; send: (next: FindState) => void }) {
  const f = normalizeFaces(state.filter.faces);
  const s = usePeople();
  useFaceSetsVersion();
  useEffect(() => { void ensurePeople(); }, []);
  const [q, setQ] = useState("");
  const [hi, setHi] = useState(0);
  const id = useId();
  const people = s.data.people.filter((p) => p.state === "active");
  const name = (pid: number) => people.find((p) => p.id === pid)?.name ?? "";
  const patch = (p: Partial<FaceFilter>) => send({ ...state, filter: { ...state.filter, faces: { ...f, ...p } } });
  const taken = new Set([...f.include, ...f.exclude]);
  const norm = (x: string) => x.trim().toLowerCase();
  const options = people.filter((p) => !taken.has(p.id) && (!q.trim() || norm(p.name).includes(norm(q)))).slice(0, 8);
  const add = (pid: number) => { patch({ include: [...f.include, pid] }); setQ(""); setHi(0); };
  const flip = (pid: number, to: "include" | "exclude") =>
    patch(to === "exclude" ? { include: f.include.filter((x) => x !== pid), exclude: [...f.exclude, pid] }
                           : { exclude: f.exclude.filter((x) => x !== pid), include: [...f.include, pid] });
  const drop = (pid: number) => patch({ include: f.include.filter((x) => x !== pid), exclude: f.exclude.filter((x) => x !== pid) });
  const chip = (pid: number, kind: "include" | "exclude") => (
    <span key={`${kind}${pid}`} className="ff-chip" data-kind={kind}>
      {kind === "exclude" && <Ban size={11} aria-hidden="true" />}
      <span>{kind === "exclude" ? t("library.find.faces.not", { name: name(pid) }) : name(pid)}</span>
      <button type="button" className="ff-chip-btn" aria-label={t(kind === "include" ? "library.find.faces.excludeOne" : "library.find.faces.includeOne", { name: name(pid) })}
              title={t(kind === "include" ? "library.find.faces.excludeOne" : "library.find.faces.includeOne", { name: name(pid) })} onClick={() => flip(pid, kind === "include" ? "exclude" : "include")}>
        {kind === "include" ? <Ban size={11} aria-hidden="true" /> : <Plus size={11} aria-hidden="true" />}
      </button>
      <button type="button" className="ff-chip-btn" aria-label={t("library.find.faces.remove", { name: name(pid) })} onClick={() => drop(pid)}><X size={11} aria-hidden="true" /></button>
    </span>
  );
  return (
    <div className="ff">
      <p className="fe-hint">{t("library.find.faces.hint")}</p>
      {(f.include.length > 0 || f.exclude.length > 0) && <div className="ff-chips">{f.include.map((p) => chip(p, "include"))}{f.exclude.map((p) => chip(p, "exclude"))}</div>}
      <input className="ff-input" role="combobox" aria-expanded={options.length > 0} aria-controls={`${id}-list`} aria-autocomplete="list" value={q} placeholder={t("library.find.faces.add")}
             aria-label={t("library.find.faces.add")} autoComplete="off" spellCheck={false} onChange={(e) => { setQ(e.target.value); setHi(0); }}
             aria-activedescendant={options.length ? `${id}-${Math.min(hi, options.length - 1)}` : undefined}
             onKeyDown={(e) => {
               if (e.key === "ArrowDown") { e.preventDefault(); setHi(Math.min(hi + 1, options.length - 1)); }
               else if (e.key === "ArrowUp") { e.preventDefault(); setHi(Math.max(hi - 1, 0)); }
               else if (e.key === "Enter" && options.length) { e.preventDefault(); add(options[Math.min(hi, options.length - 1)].id); }
             }} />
      <ul id={`${id}-list`} role="listbox" className="ff-list" aria-label={t("library.find.faces.people")}>
        {options.map((p, i) => (
          <li key={p.id} id={`${id}-${i}`} role="option" aria-selected={i === Math.min(hi, options.length - 1)} onPointerMove={() => setHi(i)} onClick={() => add(p.id)}>
            <span>{p.name}</span><span className="ff-n">{p.photos}</span>
          </li>
        ))}
        {!options.length && <li aria-disabled="true"><span className="ff-n">{people.length ? t("library.find.faces.none") : t("library.find.faces.noPeople")}</span></li>}
      </ul>
      <div className="segmented small" role="group" aria-label={t("library.find.faces.match")}>
        {([false, true] as const).map((v) => (
          <button key={String(v)} className={f.together === v ? "active" : ""} aria-pressed={f.together === v} disabled={f.include.length < 2 && v}
                  onClick={() => patch({ together: v })}>{v ? t("library.find.faces.together") : t("library.find.faces.any")}</button>
        ))}
      </div>
      <div className="meta-grid">
        <span className="k">{t("library.find.faces.noPeopleRow")}</span>
        <input type="checkbox" className="switch" aria-label={t("library.find.faces.noPeopleRow")} checked={f.none} onChange={(e) => patch({ none: e.target.checked })} />
        <span className="k">{t("library.find.faces.unnamed")}</span>
        <input type="checkbox" className="switch" aria-label={t("library.find.faces.unnamed")} checked={f.unnamed} onChange={(e) => patch({ unnamed: e.target.checked })} />
      </div>
      {(f.none || f.unnamed) && setsUnavailable() && <p className="fe-hint after">{t("library.find.faces.unavailable")}</p>}
      {JSON.stringify(f) !== JSON.stringify(NO_FACES) && <button type="button" className="people-link" onClick={() => patch({ ...NO_FACES })}>{t("library.find.faces.clear")}</button>}
    </div>
  );
}
