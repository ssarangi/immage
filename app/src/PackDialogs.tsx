// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The two dialogs of a model pack: the consent shown before any download (the
// only way to start one for a pack not yet installed), and the confirmation
// before a model is removed. Escape and Cancel close them with nothing done.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { PackInfo } from "./bindings";
import { sizeLabel, uiFmt } from "./aiPacks";
import { t } from "./i18n";

function Dialog({ title, onClose, children, foot, label }: { title: string; onClose: () => void; children: ReactNode; foot: ReactNode; label: string }) {
  const first = useRef<HTMLDivElement>(null);
  useEffect(() => {
    first.current?.querySelector<HTMLButtonElement>("button[data-first]")?.focus();
    // Escape closes this dialog and not Preferences behind it
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); onClose(); } };
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, [onClose]);
  return createPortal(
    <div className="modal-backdrop pack-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal pack-dialog" role="dialog" aria-modal="true" aria-label={label} ref={first}>
        <div className="modal-head"><h2>{title}</h2></div>
        <div className="modal-body">{children}</div>
        <div className="modal-foot">{foot}</div>
      </div>
    </div>,
    document.body,
  );
}

const Tick = () => (
  <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 7.4l2.6 2.6L11 4.4" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" /></svg>
);

export function PackConsentDialog({ info, name, what, license, onConfirm, onClose }: {
  info: PackInfo; name: string; what: string;
  /** the license text, fetched when "Read the license" is pressed */
  license: () => Promise<string>;
  onConfirm: () => void; onClose: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const size = sizeLabel(info.bytes, uiFmt);
  const read = () => {
    if (text != null) { setText(null); return; }
    license().then(setText, () => setFailed(true));
  };
  return (
    <Dialog title={t("aimodels.consent.title", { pack: name })} label={t("aimodels.consent.title", { pack: name })} onClose={onClose}
      foot={<>
        <button className="btn" data-first onClick={onClose}>{t("aimodels.act.cancel")}</button>
        <button className="btn primary" onClick={onConfirm}>{t("aimodels.consent.download", { size })}</button>
      </>}>
      <p className="pack-lead">{t("aimodels.consent.lead")}</p>
      <dl className="pack-facts">
        <dt>{t("aimodels.consent.what")}</dt><dd>{what}</dd>
        <dt>{t("aimodels.consent.size")}</dt><dd>{t("aimodels.consent.sizeFree", { size, free: sizeLabel(info.free_bytes, uiFmt) })}</dd>
        <dt>{t("aimodels.consent.from")}</dt><dd>{t("aimodels.consent.fromText", { host: info.source_host })}</dd>
        <dt>{t("aimodels.consent.license")}</dt>
        <dd>{info.license} <button type="button" className="pack-link" onClick={read} aria-expanded={text != null}>{t("aimodels.consent.readLicense")}</button>
          {failed && <span className="pack-bad"> {t("aimodels.consent.licenseFailed")}</span>}</dd>
      </dl>
      {text != null && <pre className="pack-license" tabIndex={0}>{text}</pre>}
      <ul className="pack-checks">
        {["aimodels.consent.l1", "aimodels.consent.l2", "aimodels.consent.l3"].map((k) => <li key={k}><Tick /><span>{t(k)}</span></li>)}
      </ul>
    </Dialog>
  );
}

export function PackRemoveDialog({ info, name, analysis, onConfirm, onClose }: {
  info: PackInfo; name: string; analysis: string;
  onConfirm: (keepResults: boolean) => void; onClose: () => void;
}) {
  return (
    <Dialog title={t("aimodels.remove.title", { pack: name })} label={t("aimodels.remove.title", { pack: name })} onClose={onClose}
      foot={<>
        <button className="btn" data-first onClick={onClose}>{t("aimodels.act.cancel")}</button>
        <button className="btn" onClick={() => onConfirm(true)}>{t("aimodels.remove.keep")}</button>
        <button className="btn primary" onClick={() => onConfirm(false)}>{t("aimodels.remove.all")}</button>
      </>}>
      <p className="pack-lead">{t("aimodels.remove.body", { size: sizeLabel(info.bytes_on_disk, uiFmt), analysis })}</p>
    </Dialog>
  );
}
