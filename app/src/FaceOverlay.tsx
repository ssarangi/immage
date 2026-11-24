// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The face boxes over the open photo (notes/faces-ux-spec.md 3.2). It fills a layer the size of the displayed picture
// (App.tsx places it with the mask editor's boxes, so it follows zoom and pan) and draws each face as a button at a
// percentage of it: solid for a named face, dashed for a suggestion, dotted for a face nobody has a name for. The name
// shows on hover and on keyboard focus (above the box, below it at the top edge), and always on the selected face;
// nothing here changes a size. Clicking a box opens the name box (the People view's own combobox) to name or reassign
// the face, or to say it is not them or not a face. Boxes exist only while People is on; the overlay never touches the
// photo or an export.
import { createPortal } from "react-dom";
import { useEffect, useRef, useState } from "react";
import { boxStyle, faceAria, faceTitle, labelSide } from "./faceOverlayModel";
import { t } from "./i18n";
import { NamePopover } from "./PeopleName";
import { ensurePeople, ignoreTiles, notPerson, nameTiles, usePeople, type NameTarget, type PhotoFace } from "./peopleStore";
import { usePhotoFaces } from "./usePhotoFaces";

export function FaceOverlay({ path, selected, onSelect }: { path: string; selected: number | null; onSelect: (face: number | null) => void }) {
  const { faces } = usePhotoFaces(path, true);
  const people = usePeople().data.people;
  useEffect(() => { void ensurePeople(); }, []);
  const [pick, setPick] = useState<{ face: PhotoFace; rect: DOMRect } | null>(null);
  const refs = useRef(new Map<number, HTMLButtonElement>());
  // the selected face (from the Library card) scrolls nothing and opens nothing: it only keeps its name showing
  useEffect(() => { setPick(null); }, [path]);
  const act = (f: PhotoFace, to: NameTarget) => { setPick(null); void nameTiles([`f:${f.face}`], to); };
  return (
    <div className="face-layer" role="group" aria-label={t("people.overlay.label")}>
      {faces.map((f, i) => (
        <button key={f.face} type="button" ref={(el) => { if (el) refs.current.set(f.face, el); else refs.current.delete(f.face); }}
                className={`face-box${selected === f.face ? " selected" : ""}`} data-state={f.state} data-side={labelSide(f)} style={boxStyle(f)}
                aria-label={faceAria(f, i, faces.length, t)} aria-haspopup="dialog" aria-expanded={pick?.face.face === f.face}
                onFocus={() => onSelect(f.face)} onPointerDown={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}
                onClick={(e) => { e.stopPropagation(); onSelect(f.face); setPick({ face: f, rect: e.currentTarget.getBoundingClientRect() }); }}>
          <span className="face-label" aria-hidden="true">{faceTitle(f, t)}</span>
        </button>
      ))}
      {pick && createPortal(
        <div className="face-pop" onPointerDown={(e) => e.stopPropagation()} style={{ position: "fixed", inset: 0, pointerEvents: "none", zIndex: 60 }}>
          <div style={{ pointerEvents: "auto" }}
               onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); setPick(null); refs.current.get(pick.face.face)?.focus(); } }}>
            <NamePopover o={{ pos: () => ({ x: pick.rect.left, y: pick.rect.top }), scrollTop: 0, width: window.innerWidth, height: window.innerHeight,
                              g: { cols: 1, tile: Math.max(0, pick.rect.height - 36), rowH: 0, pad: 0, gap: 0 } }}
                         index={0} count={1} group={false} seed="" people={people} onPick={(to) => act(pick.face, to)} onClose={() => setPick(null)}
                         extra={<div className="face-pop-actions">
                           {pick.face.person !== null && pick.face.name && (
                             <button type="button" className="people-btn" onClick={() => { setPick(null); void notPerson([`f:${pick.face.face}`], new Map([[`f:${pick.face.face}`, pick.face.person]]), new Map([[pick.face.person!, pick.face.name!]])); }}>
                               {t("people.act.notNamed", { name: pick.face.name })}</button>
                           )}
                           <button type="button" className="people-btn" onClick={() => { setPick(null); void ignoreTiles([`f:${pick.face.face}`]); }}>{t("people.act.notFace")}</button>
                         </div>} />
          </div>
        </div>, document.body)}
    </div>
  );
}
