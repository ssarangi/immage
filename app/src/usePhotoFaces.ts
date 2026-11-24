// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The faces of one photo, for the overlay and the Library card: read when the photo changes and again when anything
// about faces changed (naming, merging, a scan finding more), ordered left to right. Nothing is asked while People is off.
import { useEffect, useState } from "react";
import { orderFaces } from "./faceOverlayModel";
import { peopleApi, usePeople, type PhotoFace } from "./peopleStore";

export function usePhotoFaces(path: string | null, on: boolean): { faces: PhotoFace[]; loading: boolean } {
  const tick = usePeople().tick;
  const [state, setState] = useState<{ path: string | null; faces: PhotoFace[] }>({ path: null, faces: [] });
  useEffect(() => {
    if (!path || !on) { setState({ path: null, faces: [] }); return; }
    let alive = true;
    peopleApi().photoFaces(path).then((list) => { if (alive) setState({ path, faces: orderFaces(list) }); },
      () => { if (alive) setState({ path, faces: [] }); });
    return () => { alive = false; };
  }, [path, on, tick]);
  // faces of the photo before this one are never shown for this one
  return { faces: state.path === path ? state.faces : [], loading: !!path && on && state.path !== path };
}
