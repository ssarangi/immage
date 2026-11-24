// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > Faces (notes/faces-ux-spec.md 6): the People switch (turning it on for the first time is the People
// view's consent card, never a silent enable), the model pack, what the recognizer and the optional eye model are, the
// network folders left out, the numbers the face store holds, and Delete all face data. Every row has the height it
// will have when its numbers arrive, so a refresh of the numbers moves nothing.
import "./people.css";
import { useCallback, useEffect, useRef, useState } from "react";
import type { FacesSettings, FacesStats } from "./bindings";
import { commands } from "./commands";
import { packView, uiFmt } from "./aiPacks";
import { usePacks } from "./aiPacks";
import { formatNumber, t } from "./i18n";
import { PackCard } from "./PackCard";
import { PackConsentDialog } from "./PackDialogs";
import { Modal } from "./PeopleView";
import { PrefCard, PrefRow, PrefSwitch } from "./PrefsUI";
import { settings } from "./settings";
import { scanNotes } from "./peopleModel";
import { CONSENT_KEY, deleteAllFaceData, failedPhotos, peopleApi, refreshStatus, resumePeople, turnOff, usePeople } from "./peopleStore";

const MB = 1e6;
const size = (bytes: number) => bytes >= 1e9 ? `${formatNumber(bytes / 1e9, { maximumFractionDigits: 1, minimumFractionDigits: 1 })} GB`
  : bytes >= MB ? `${formatNumber(Math.round(bytes / MB))} MB` : `${formatNumber(Math.round(bytes / 1000))} KB`;

type Dialog = { kind: "resume" } | { kind: "delete" } | { kind: "reprocess" } | { kind: "failed"; paths: string[] } | null;

export function FacesPrefs({ onOpenPeople }: { onOpenPeople?: () => void }) {
  const s = usePeople();
  const st = s.status;
  const enabled = !!st?.enabled, installed = !!st?.installed;
  const { packs, refresh, setPacks } = usePacks();
  const pack = packs?.find((p) => p.id === "faces") ?? null;
  const [stats, setStats] = useState<FacesStats | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [flash, setFlash] = useState<string | null>(null);
  // what the recognition settings are now (re-read after each change), a thoroughness change waiting for its confirmation, and the note after a reprocess
  const [cfg, setCfg] = useState<FacesSettings | null>(null);
  const [pendingSize, setPendingSize] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const alive = useRef(true);

  // the numbers: read when the page opens, every few seconds while it is open, and when the faces change
  const read = useCallback(() => {
    peopleApi().stats().then((x) => { if (alive.current) setStats(x); }, () => {});
  }, []);
  const readCfg = useCallback(() => { peopleApi().settings().then((x) => { if (alive.current) setCfg(x); }, () => {}); }, []);
  useEffect(() => {
    alive.current = true;
    read();
    readCfg();
    const timer = setInterval(read, 5000);
    return () => { alive.current = false; clearInterval(timer); };
  }, [read, readCfg, s.tick]);

  const strict = cfg?.strictness ?? "balanced", detail = cfg?.detect_size === 1600 ? 1600 : 1024;
  const setStrict = (level: string) => { if (level !== strict) void peopleApi().setStrictness(level).then((x) => setCfg(x), () => readCfg()); };
  const askDetail = (size: number) => setPendingSize(size === detail ? null : size);
  const applyDetail = () => {
    const size = pendingSize;
    setPendingSize(null);
    if (size !== null) void peopleApi().setDetectSize(size).then((x) => setCfg(x), () => readCfg());
  };
  const run = async (job: () => Promise<unknown>) => {
    setBusy(true);
    try { await job(); } catch { /* the pack's own line shows the failure */ } finally { setBusy(false); void refresh(); read(); }
  };
  const onPack = (a: string) => {
    if (!pack) return;
    if (a === "download") setConsent(true);
    else if (a === "cancel") void commands.packCancel("faces").catch(() => {});
    else if (a === "resume" || a === "repair") void run(async () => { setPacks((l) => l && l.map((p) => (p.id === "faces" ? { ...p, state: "downloading", error: null } : p))); await commands.packDownload("faces", false); });
    else if (a === "check") void run(async () => {
      const r = await commands.packVerify("faces");
      setFlash(r.failed.length ? t("aimodels.check.bad", { bad: r.failed.length, n: r.files }) : t("aimodels.check.ok", { n: r.files }));
      setTimeout(() => setFlash(null), 6000);
    });
    else if (a === "removePartial") void run(() => commands.packRemove("faces", true, true));
  };

  // the switch: off is one click (the data stays); on again after a first time asks once; the first time is People's own card
  const consented = !!settings.get(CONSENT_KEY);
  const onSwitch = (on: boolean) => {
    if (!on) { void turnOff(); return; }
    if (!consented) { onOpenPeople?.(); return; }
    setDialog({ kind: "resume" });
  };
  const reason = !enabled && !installed ? t("aimodels.switch.needModel") : !enabled && !consented ? t("prefs.faces.firstTime") : "";

  const view = pack ? packView(pack, t, uiFmt, !!flash) : null;
  const notes = scanNotes(st);
  const n = (v: number | undefined) => (v === undefined ? "-" : formatNumber(v));
  const sr = stats?.store;
  const recognizer = stats && stats.recognizer === "arcface" && stats.arcface_available ? t("prefs.faces.recognizerArcface") : t("prefs.faces.recognizerSface");
  const eyes = (stats?.eyes_available ?? st?.eyes_available) ? t("prefs.faces.eyesOn") : t("prefs.faces.eyesOff");

  return (
    <>
      <PrefCard title={t("prefs.faces.people")} card="faces/people">
        <PrefRow id="People" label={t("prefs.faces.switch")} hint={t("prefs.faces.switchHint")}>
          <PrefSwitch checked={enabled} disabled={busy || (!enabled && !installed)} onChange={onSwitch} aria-label={t("prefs.faces.switch")} />
        </PrefRow>
        <p className="faces-line" role="status">{reason}</p>
        <PrefRow id="Recognizer" label={t("prefs.faces.recognizer")}><span className="faces-val">{recognizer}</span></PrefRow>
        <PrefRow id="Eye detection" label={t("prefs.faces.eyes")}><span className="faces-val faces-wrap">{eyes}</span></PrefRow>
        <p className="faces-line" role="status">{notes.network > 0 ? t("prefs.faces.network", { n: notes.network, count: formatNumber(notes.network) }) : ""}</p>
        {onOpenPeople && <div className="faces-act"><button type="button" className="btn small" onClick={onOpenPeople}>{t("prefs.faces.open")}</button></div>}
      </PrefCard>

      <PrefCard title={t("prefs.faces.recognition")} card="faces/recognition">
        <PrefRow id="Same person strictness" label={t("prefs.faces.strictness")} hint={t("prefs.faces.strictnessHint")}>
          <div className="segmented small" role="group" aria-label={t("prefs.faces.strictness")}>
            {(["loose", "balanced", "strict"] as const).map((v) => (
              <button key={v} className={strict === v ? "active" : ""} aria-pressed={strict === v} disabled={!cfg} onClick={() => setStrict(v)}>{t(`prefs.faces.strict.${v}`)}</button>
            ))}
          </div>
        </PrefRow>
        <PrefRow id="Scan thoroughness" label={t("prefs.faces.detail")} hint={t("prefs.faces.detailHint")}>
          <div className="segmented small" role="group" aria-label={t("prefs.faces.detail")}>
            {([1024, 1600] as const).map((v) => (
              <button key={v} className={detail === v ? "active" : ""} aria-pressed={detail === v} disabled={!cfg} onClick={() => askDetail(v)}>{t(v === 1024 ? "prefs.faces.detail.balanced" : "prefs.faces.detail.thorough")}</button>
            ))}
          </div>
        </PrefRow>
        <div className="faces-confirm" role="status">
          {pendingSize !== null && <>
            <span>{t("prefs.faces.rescanNote")}</span>
            <button type="button" className="btn small primary" onClick={applyDetail}>{t("prefs.faces.change")}</button>
            <button type="button" className="btn small" onClick={() => setPendingSize(null)}>{t("aimodels.act.cancel")}</button>
          </>}
        </div>
        <p className="faces-line" role="status">{note}</p>
        <div className="faces-act">
          <button type="button" className="btn small" disabled={busy || !enabled || !installed} onClick={() => setDialog({ kind: "reprocess" })}>{t("prefs.faces.reprocess")}</button>
        </div>
      </PrefCard>

      <PrefCard title={t("prefs.faces.model")} card="faces/model">
        {pack && view ? (
          <PackCard info={pack} view={view} name={t("aimodels.faces.name")} does={t("aimodels.faces.does")} photos={t("aimodels.faces.photos")} card="faces/pack"
                    busy={busy} flash={flash} onAction={(a) => onPack(a)} />
        ) : <p className="faces-line">{t("people.consent.reading")}</p>}
      </PrefCard>

      <PrefCard title={t("prefs.faces.data")} card="faces/data">
        <dl className="faces-stats" aria-label={t("prefs.faces.data")}>
          <dt>{t("prefs.faces.stat.photos")}</dt><dd>{n(sr?.photos)}</dd>
          <dt>{t("prefs.faces.stat.faces")}</dt><dd>{n(sr?.faces)}</dd>
          <dt>{t("prefs.faces.stat.people")}</dt><dd>{n(sr?.people)}</dd>
          <dt>{t("prefs.faces.stat.groups")}</dt><dd>{n(sr?.groups)}</dd>
          <dt>{t("prefs.faces.stat.failed")}</dt>
          <dd>{n(sr?.failed)}{sr && sr.failed > 0 && <button type="button" className="people-link faces-show" onClick={() => void failedPhotos().then((paths) => setDialog({ kind: "failed", paths }), () => setDialog({ kind: "failed", paths: [] }))}>{t("prefs.faces.show")}</button>}</dd>
          <dt>{t("prefs.faces.stat.db")}</dt><dd>{sr ? size(sr.db_bytes) : "-"}</dd>
          <dt>{t("prefs.faces.stat.crops")}</dt><dd>{stats ? size(stats.crops_bytes) : "-"}</dd>
        </dl>
        <p className="faces-line faces-quiet">{t("prefs.faces.local")}</p>
        <div className="faces-act">
          <button type="button" className="btn small" disabled={busy || !sr || (sr.faces === 0 && sr.people === 0)} onClick={() => setDialog({ kind: "delete" })}>{t("people.delete.act")}</button>
        </div>
      </PrefCard>

      {consent && pack && (
        <PackConsentDialog info={pack} name={t("aimodels.faces.name")} what={t("aimodels.faces.what", { n: 2 })} license={() => commands.packLicense("faces")}
                           onConfirm={() => { setConsent(false); void run(() => commands.packDownload("faces", true)); }} onClose={() => setConsent(false)} />
      )}
      {dialog?.kind === "resume" && (
        <Modal title={t("prefs.faces.resumeTitle")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>
                 <button type="button" className="btn primary" onClick={() => { setDialog(null); void run(() => resumePeople()); }}>{t("prefs.faces.resume")}</button></>}>
          <p className="pack-lead">{t("prefs.faces.resumeBody")}</p>
        </Modal>
      )}
      {dialog?.kind === "reprocess" && (
        <Modal title={t("prefs.faces.reprocessTitle")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>
                 <button type="button" className="btn primary" onClick={() => {
                   setDialog(null);
                   void run(async () => {
                     const n = await peopleApi().reprocess();
                     setNote(t("prefs.faces.reprocessing", { n, count: formatNumber(n) }));
                     await refreshStatus();
                   });
                 }}>{t("prefs.faces.reprocessGo")}</button></>}>
          <p className="pack-lead">{t("prefs.faces.reprocessBody")}</p>
        </Modal>
      )}
      {dialog?.kind === "delete" && (
        <Modal title={t("people.delete.title")} onClose={() => setDialog(null)}
               foot={<><button type="button" className="btn" data-first onClick={() => setDialog(null)}>{t("aimodels.act.cancel")}</button>
                 <button type="button" className="btn primary" onClick={() => { setDialog(null); void run(() => deleteAllFaceData()); }}>{t("people.delete.confirm")}</button></>}>
          <p className="pack-lead">{t("prefs.faces.deleteBody", { people: formatNumber(sr?.people ?? 0), faces: formatNumber(sr?.faces ?? 0), size: size((sr?.db_bytes ?? 0) + (stats?.crops_bytes ?? 0)) })}</p>
        </Modal>
      )}
      {dialog?.kind === "failed" && (
        <Modal title={t("people.failed.title")} onClose={() => setDialog(null)}
               foot={<button type="button" className="btn primary" data-first onClick={() => setDialog(null)}>{t("people.failed.close")}</button>}>
          <p className="pack-lead">{t("people.failed.body")}</p>
          <ul className="people-failed">{dialog.paths.map((x) => <li key={x} title={x}>{x}</li>)}</ul>
        </Modal>
      )}
    </>
  );
}
