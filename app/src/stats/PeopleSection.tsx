// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Statistics > People (shown only while Faces is on): the people with the most photos, a row each with their face, their name,
// how many photos and a bar. A photo counts once however many files its shot has, and the counts are the whole library's: the
// filters above do not narrow them (the statistics model has no person filter, so a row opens the person's page instead).
// The card has the height of its rows from the first paint (a skeleton of the same rows), and it is the last section, so its
// answer arriving moves nothing above it.
import { useEffect, useState } from "react";
import type { PersonRow } from "../bindings";
import { formatNumber, t } from "../i18n";
import { Crop } from "../PeopleGrid";
import { topPeople } from "../peopleModel";
import { peopleApi, usePeople } from "../peopleStore";
import { SectionHead } from "./Section";

const ROWS = 8;   // the card's height is these rows, whatever it holds (statistics.css .st-people)

export function PeopleSection({ onOpenPerson }: { onOpenPerson?: (id: number) => void }) {
  const tick = usePeople().tick;
  const [people, setPeople] = useState<PersonRow[] | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let alive = true;
    peopleApi().people().then((x) => { if (alive) { setPeople(x); setFailed(false); } }, () => { if (alive) setFailed(true); });
    return () => { alive = false; };
  }, [tick]);
  const rows = people ? topPeople(people) : [];
  const max = Math.max(1, ...rows.map((p) => p.photos));
  return (
    <section className="st-sec" id="people" aria-label={t("stats.people.title")}>
      <SectionHead eyebrow={t("stats.people.eyebrow")} title={t("stats.people.title")} em={t("stats.people.titleEm")} lede={t("stats.people.lede")} />
      <div className="st-card st-people" aria-busy={people === null && !failed}>
        {rows.map((p) => (
          <button key={p.id} type="button" className="st-people-row" onClick={() => onOpenPerson?.(p.id)}
                  aria-label={t("stats.people.row", { name: p.name, n: p.photos, count: formatNumber(p.photos) })}
                  title={t("stats.people.open", { name: p.name })}>
            <Crop face={p.cover_face_id} tile={36} className="st-people-face" />
            <span className="st-people-name">{p.name}</span>
            <span className="st-people-bar" aria-hidden="true"><i style={{ width: `${Math.round((p.photos / max) * 100)}%` }} /></span>
            <span className="st-people-n">{formatNumber(p.photos)}</span>
          </button>
        ))}
        {people === null && !failed && Array.from({ length: ROWS }, (_, i) => <div key={i} className="st-people-row st-people-skel" aria-hidden="true"><i /></div>)}
        {(failed || (people !== null && rows.length === 0)) && <p className="st-people-empty">{failed ? t("stats.people.error") : t("stats.people.empty")}</p>}
      </div>
    </section>
  );
}
