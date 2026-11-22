// SPDX-License-Identifier: GPL-3.0-or-later
import { Camera, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { easeZoom, ZOOM_EASE } from "./zoom";
import { loadReadAhead } from "./Preferences";
import { BanIcon, StarIcon } from "./icons";
import { commands, type CullChange } from "./commands";
import { CullLightbox, LightboxHead, StageArrows } from "./CullLightbox";
import { CullOverlay, CullSide, NO_SEL, overrideDecision, useCritique, type Sel, type Tab } from "./CullInspector";
import { reviewKey } from "./cullModel";
import { cullState, useDecision, useReviewing } from "./cullStore";
import { cullingKey, fallbackPhoto, pickCritiqueTarget, pickCullingTarget, fitLightbox, keywordTargets, previewNeighbors, PreviewQueue, wheelZoom } from "./culling";
import { LABEL_COLORS, fetchCachedThumb, fetchThumb, fileKind, type ImageEntry } from "./library";
import { KeywordingPanel, parseKeyword } from "./Keywording";
import { ZoomBar } from "./Toolbar";
import { formatNumber, t } from "./i18n";
import type { SceneLightbox } from "./SceneLightbox";

export function GridCulling({ images: gridImages, allImages, selection, anchor, thumbnail, activeEditorId, prepare, onEdit, onMeta, onKeywords, onCullChanges, onOpenCullPrefs, scene }: {
  images: ImageEntry[]; allImages: ImageEntry[]; selection: Set<number>; anchor: number | null;
  thumbnail: (id: number) => ImageBitmap | undefined;
  activeEditorId?: number;
  prepare: (id: number) => Promise<void>; onEdit: (image: ImageEntry) => void;
  onMeta: (image: ImageEntry, fields: Partial<ImageEntry>) => void;
  onKeywords: (id: number, keywords: string[]) => void;
  onCullChanges: (changes: CullChange[]) => void;   // library entries an override rewrote
  onOpenCullPrefs: () => void;                      // Preferences > Culling
  scene?: SceneLightbox | null;                     // the scene view: this lightbox over one scene's frames
}) {
  // in the scene view the lightbox steps through the scene's frames, not the grid's photos
  const images = scene ? scene.frames : gridImages;
  const zoomAnchor = useRef<{ix: number; iy: number; sx: number; sy: number} | null>(null);
  const space = useRef(false);
  const [spaceHeld, setSpaceHeld] = useState(false);
  const [panning, setPanning] = useState(false);
  const panClick = useRef(false);
  const [paintSize, setPaintSize] = useState({width: 300, height: 150});
  const direction = useRef<1 | -1>(1);
  const queue = useRef(new PreviewQueue());
  const previews = useRef(new Map<string, ImageBitmap>());
  const pendingPreviews = useRef(new Map<string, Promise<ImageBitmap | null>>());
  const previewEpoch = useRef(0);
  const [windowSize, setWindowSize] = useState({width: window.innerWidth, height: window.innerHeight});
  const animation = useRef<{ frame: number; width: number; height: number } | null>(null);
  const targetSize = useRef({ width: 300, height: 150 });
  const zooming = useRef(false);
  const viewport = useRef<HTMLDivElement>(null);
  const keywordGeneration = useRef(0);
  const [retry, setRetry] = useState(0);
  const [detail, setDetail] = useState(false);
  const [cameraPreviews, setCameraPreviews] = useState(false);
  const [photo, setPhoto] = useState<number | null>(null);
  const [targets, setTargets] = useState<ImageEntry[]>([]);
  const [lists, setLists] = useState<string[][]>([]);
  const [known, setKnown] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [writing, setWriting] = useState(false);
  const [busy, setBusy] = useState(false);
  const canvas = useRef<HTMLCanvasElement>(null);
  // null is Fit; a number is the zoom multiplier (1 means 100%).
  const [zoom, setZoomState] = useState<number | null>(null);
  const zoomRef = useRef<number | null>(null);
  zoomRef.current = zoom;
  const setZoom = (value: number | null) => { zoomRef.current = value; setZoomState(value); };
  const detailed = zoom !== null;
  const cameraFit = cameraPreviews && !detailed;
  const current = images.find(i => i.imgid === photo) ?? (scene ? images.find(i => i.imgid === scene.startId) : undefined);
  // auto cull under review: the inspector beside the photo, its overlay over it
  const reviewing = useReviewing();
  const [tabChosen, setTabChosen] = useState<Tab | null>(null);   // null: Cull while reviewing, else Critique
  const tab: Tab = tabChosen ?? (scene ? "scene" : reviewing ? "cull" : "critique");
  const [sel, setSel] = useState<Sel>(NO_SEL);
  const decision = useDecision(current?.path ?? "", current?.version);
  const crit = useCritique(current);
  // a critique asked for from the menu, the right-click menu or the top bar starts
  // once its photo is open
  const wantCritique = useRef<number | null>(null);
  useEffect(() => { setSel(NO_SEL); }, [photo]);
  useEffect(() => {
    if (current && wantCritique.current === current.imgid) { wantCritique.current = null; crit.run(false); }
  }, [current?.imgid]);
  const openPrefs = () => { setPhoto(null); onOpenCullPrefs(); };
  // The frame is fixed for the window size, independent of photo or preview
  // dimensions, so swapping photos and zooming never moves the toolbar.
  const fitted = fitLightbox(windowSize.width, windowSize.height);   // the lightbox follows it in CSS; this re-fits the photo on resize
  const zoomShow = useRef<(scale: number, isFit: boolean) => void>(() => {});
  const fitScale = () => {
    const box = viewport.current;
    if (!box || !current) return 1;
    const dpr = window.devicePixelRatio || 1;
    const width = (detail ? paintSize.width : current.width || paintSize.width) / dpr;
    const height = (detail ? paintSize.height : current.height || paintSize.height) / dpr;
    return Math.min(box.clientWidth / Math.max(1, width), box.clientHeight / Math.max(1, height));
  };
  useLayoutEffect(() => {
    zoomShow.current(zoom ?? fitScale(), zoom === null);
  });
  useEffect(() => {
    const resize = () => setWindowSize({width: window.innerWidth, height: window.innerHeight});
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, []);
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!current) return;
      e.preventDefault();
      const image = canvas.current?.getBoundingClientRect();
      const bounds = el.getBoundingClientRect();
      if (image && image.width && image.height) zoomAnchor.current = {
        ix: Math.max(0, Math.min(1, (e.clientX - image.left) / image.width)),
        iy: Math.max(0, Math.min(1, (e.clientY - image.top) / image.height)),
        sx: e.clientX - bounds.left, sy: e.clientY - bounds.top,
      };
      const fit = fitScale();
      setZoom(wheelZoom(zoomRef.current ?? fit, e.deltaY, e.ctrlKey, fit));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [current?.imgid, current?.width, current?.height, detail, paintSize]);
  const previousIndex = useRef(0);
  useEffect(() => { if (current) previousIndex.current = images.indexOf(current); else if (photo !== null) setPhoto(fallbackPhoto(images, previousIndex.current)); }, [images, photo]);
  // a scene opened (or another scene chosen) shows its first frame; closing the lightbox closes the scene
  const sceneShown = useRef(false);
  useEffect(() => {
    if (scene) { direction.current = 1; setPhoto(scene.startId); }
    else if (sceneShown.current) { sceneShown.current = false; setPhoto(null); }
  }, [scene?.key]);   // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (scene && photo !== null) sceneShown.current = true;
    else if (scene && photo === null && sceneShown.current) { sceneShown.current = false; scene.onClose(); }
  }, [photo]);   // eslint-disable-line react-hooks/exhaustive-deps
  const keywords = async (items: ImageEntry[]) => {
    const generation = ++keywordGeneration.current;
    setTargets(items); setLists([]); setError("");
    try {
      const [meta, all] = await Promise.all([Promise.all(items.map(i => commands.meta(i.imgid))), commands.keywords()]);
      if (generation !== keywordGeneration.current) return;
      setLists(meta.map(m => m.image.keywords)); setKnown(all);
    } catch (e) { if (generation === keywordGeneration.current) setError(String(e)); }
  };
  useEffect(() => {
    const open = (event: Event) => {
      const ids = (event as CustomEvent<number[]>).detail;
      void keywords(allImages.filter(i => ids.includes(i.imgid) && !i.remote));
    };
    window.addEventListener("grid-keywords", open);
    return () => window.removeEventListener("grid-keywords", open);
  });
  // The F key, the toolbar button, the menu and a thumbnail's icon all open the
  // lightbox through here: the photo asked for, else the hovered one, else the
  // anchor or selection.
  const hoveredPhoto = () => document.querySelector(".filmstrip.grid .thumb:hover")?.getAttribute("data-imgid");
  const cullingTarget = (asked?: number) => pickCullingTarget(images, asked, hoveredPhoto(), anchor, selection);
  useEffect(() => {
    const open = (event: Event) => {
      if (document.querySelector(".modal-backdrop")) return;
      // the button and the menu always open something: with no photo hovered or
      // selected, the first one
      const target = cullingTarget((event as CustomEvent<{ imgid?: number }>).detail?.imgid) ?? images[0];
      if (!target) return;
      direction.current = 1; setPhoto(target.imgid);
    };
    window.addEventListener("grid-culling-open", open);
    return () => window.removeEventListener("grid-culling-open", open);
  });
  // P keeps, R rejects, B makes the photo its set's best, in review (the
  // decisions go through the inspector's overrides, so the buttons and keys agree)
  useEffect(() => {
    const review = (e: KeyboardEvent) => {
      if (!current || !cullState().open || !cullState().plan || targets.length) return;
      const focused = e.target as HTMLElement | null;
      const action = reviewKey({ key: e.key, ctrlKey: e.ctrlKey, metaKey: e.metaKey, altKey: e.altKey,
        typing: !!focused?.closest('input:not([type="range"]),textarea,select,[contenteditable=true]') });
      if (!action || action.type === "step" || !decision) return;
      e.preventDefault(); e.stopImmediatePropagation();
      void overrideDecision(decision, action.type === "reject" ? "reject" : "keep", action.type === "best" ? true : null, onCullChanges)
        .catch(() => {});
    };
    window.addEventListener("keydown", review, true);
    return () => window.removeEventListener("keydown", review, true);
  });
  useEffect(() => {
    const open = (event: Event) => {
      if (document.querySelector(".modal-backdrop")) return;
      const asked = (event as CustomEvent<{ imgid?: number } | null>).detail?.imgid;
      const target = pickCritiqueTarget(images, asked, hoveredPhoto(), anchor, selection, activeEditorId);
      if (!target) return;
      direction.current = 1; setTabChosen("critique"); wantCritique.current = target.imgid; setPhoto(target.imgid);
    };
    window.addEventListener("grid-critique-open", open);
    return () => window.removeEventListener("grid-critique-open", open);
  });
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      const focused = e.target as HTMLElement | null;
      const adjustSlider = !!focused?.closest('input[type="range"]') && (k === "arrowleft" || k === "arrowright");
      const chosen = keywordTargets(allImages, selection);
      const target = pickCullingTarget(images, undefined, document.querySelector(".filmstrip.grid .thumb:hover")?.getAttribute("data-imgid"), anchor, selection);
      const { kind, action } = cullingKey(k, {
        photo, keywordsOpen: targets.length > 0, writing,
        blocked: e.ctrlKey || e.metaKey || e.altKey || adjustSlider
          || !!focused?.closest('input:not([type="range"]),textarea,[contenteditable=true]'),
        modalOpen: !!document.querySelector(".modal-backdrop"),
        hasTarget: !!target || chosen.length > 0, hasCurrent: !!current,
        scene: !!scene && photo !== null, shift: e.shiftKey,
      });
      if (kind === "ignore") return;
      e.stopImmediatePropagation();
      if (kind === "swallow") return;
      e.preventDefault();
      switch (action?.type) {
        case "closeKeywords": ++keywordGeneration.current; setTargets([]); break;
        case "holdSpace": space.current = true; setSpaceHeld(true); break;
        case "closeLightbox": if (sel.pinned !== null) setSel({ ...sel, pinned: null }); else setPhoto(null); break;
        case "toggleLightbox": direction.current = 1; setPhoto(photo ? null : target?.imgid ?? null); break;
        case "keywords": void keywords(current ? [current] : keywordTargets(allImages, selection, target?.imgid)); break;
        case "toggleZoom": zoomAnchor.current = null; setZoom(zoomRef.current === null ? 1 : null); break;
        case "step": {
          direction.current = action.by;
          const next = images[images.indexOf(current!) + direction.current];
          if (next) setPhoto(next.imgid);
          break;
        }
        case "rate": onMeta(current!, { rating: action.value, rejected: false }); break;
        case "label": onMeta(current!, { labels: current!.labels ^ (1 << action.bit) }); break;
        case "toggleReject":
          if (scene) scene.decide(current!, current!.rejected ? "undecided" : "rejected");
          else onMeta(current!, { rejected: !current!.rejected });
          break;
        case "decide": scene?.decide(current!, action.state); break;
        case "scene": scene?.onScene(action.by); break;
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  });
  useEffect(() => {
    const release = () => { space.current = false; setSpaceHeld(false); };
    const up = (e: KeyboardEvent) => { if (e.key === " ") release(); };
    window.addEventListener("keyup", up); window.addEventListener("blur", release);
    return () => { window.removeEventListener("keyup", up); window.removeEventListener("blur", release); };
  }, []);
  useEffect(() => () => {
    const state = animation.current;
    if (state) cancelAnimationFrame(state.frame);
    animation.current = null;
  }, [photo]);
  // Retarget the active animation when better pixels arrive; do not restart it
  // from the current rectangle. That restart made previews visibly bounce.
  const targetWidth = detailed ? (detail ? paintSize.width : current?.width || paintSize.width) / devicePixelRatio * zoom! : paintSize.width;
  const targetHeight = detailed ? (detail ? paintSize.height : current?.height || paintSize.height) / devicePixelRatio * zoom! : paintSize.height;
  const box = viewport.current;
  const factor = detailed ? 1 : box
    ? Math.min(box.clientWidth / Math.max(1, targetWidth), box.clientHeight / Math.max(1, targetHeight)) : 1;
  targetSize.current = { width: Math.max(1, targetWidth * factor), height: Math.max(1, targetHeight * factor) };
  zooming.current = detailed;
  useEffect(() => {
    const image = canvas.current, viewportBox = viewport.current;
    if (!image || !viewportBox || !current) return;
    const w = detailed ? (detail ? paintSize.width : current.width || paintSize.width) / devicePixelRatio * zoom! : paintSize.width;
    const h = detailed ? (detail ? paintSize.height : current.height || paintSize.height) / devicePixelRatio * zoom! : paintSize.height;
    const scale = detailed ? 1 : Math.min(viewportBox.clientWidth / Math.max(1, w),
      viewportBox.clientHeight / Math.max(1, h));
    targetSize.current = { width: Math.max(1, w * scale), height: Math.max(1, h * scale) };
    if (animation.current) return;
    const rect = image.getBoundingClientRect();
    animation.current = { frame: 0, width: rect.width || targetSize.current.width,
      height: rect.height || targetSize.current.height };
    const step = () => {
      const state = animation.current;
      if (!state) return;
      const target = targetSize.current;
      state.width = easeZoom(state.width, target.width);
      state.height = easeZoom(state.height, target.height);
      image.style.width = state.width + "px"; image.style.height = state.height + "px";
      if (!zooming.current) { viewportBox.scrollLeft *= 1 - ZOOM_EASE; viewportBox.scrollTop *= 1 - ZOOM_EASE; }
      else {
        const anchor = zoomAnchor.current ?? {ix:.5, iy:.5, sx:viewportBox.clientWidth/2, sy:viewportBox.clientHeight/2};
        viewportBox.scrollLeft = state.width * anchor.ix - anchor.sx + Math.max(0, (viewportBox.clientWidth - state.width) / 2);
        viewportBox.scrollTop = state.height * anchor.iy - anchor.sy + Math.max(0, (viewportBox.clientHeight - state.height) / 2);
      }
      if (state.width !== target.width || state.height !== target.height) state.frame = requestAnimationFrame(step);
      else animation.current = null;
    };
    animation.current.frame = requestAnimationFrame(step);
  }, [zoom, detail, paintSize, fitted.width, fitted.height, photo]);
  useLayoutEffect(() => {
    if (!current) {
      ++previewEpoch.current;
      pendingPreviews.current.clear();
      for (const bitmap of previews.current.values()) bitmap.close();
      previews.current.clear();
      return;
    }
    let stale = false;
    const epoch = previewEpoch.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const q = queue.current;
    q.replace();
    setDetail(false); setBusy(true); setError("");
    if (!detailed && canvas.current) canvas.current.getContext("2d")?.clearRect(0, 0, canvas.current.width, canvas.current.height);
    const box = viewport.current;
    const size = detailed ? 0 : Math.ceil(Math.max(box?.clientWidth ?? window.innerWidth * .86,
      box?.clientHeight ?? window.innerHeight * .86) * devicePixelRatio);

    const draw = (bitmap: ImageBitmap) => {
      if (!stale && canvas.current) {
        if (!detailed && viewport.current && canvas.current) {
          const scale = Math.min(viewport.current.clientWidth / bitmap.width, viewport.current.clientHeight / bitmap.height);
          canvas.current.style.width = bitmap.width * scale + "px";
          canvas.current.style.height = bitmap.height * scale + "px";
        }
        setPaintSize({width: bitmap.width, height: bitmap.height});
        canvas.current.width = bitmap.width; canvas.current.height = bitmap.height;
        canvas.current.getContext("2d")?.drawImage(bitmap, 0, 0);
      }
    };
    // Borrow the grid bitmap only for this synchronous draw; its loader owns it.
    // A virtual copy is looked up by imgid, never by the shared original path.
    const small = thumbnail(current.imgid);
    if (!detailed && small && canvas.current) {
      if (!detailed && viewport.current && canvas.current) {
        const scale = Math.min(viewport.current.clientWidth / small.width, viewport.current.clientHeight / small.height);
        canvas.current.style.width = small.width * scale + "px";
        canvas.current.style.height = small.height * scale + "px";
      }
      setPaintSize({width: small.width, height: small.height});
      canvas.current.width = small.width; canvas.current.height = small.height;
      canvas.current.getContext("2d")?.drawImage(small, 0, 0);
    }
    const cameraPreviewFor = (entry: ImageEntry, requested: number) =>
      cameraFit && requested > 0 && entry.edited && fileKind(entry.filename) === "RAW";
    const key = (entry: ImageEntry, requested: number) =>
      JSON.stringify([entry.imgid, entry.path, entry.version, entry.edited, requested, cameraPreviewFor(entry, requested)]);
    const load = async (entry: ImageEntry, requested: number, foreground = false) => {
      const id = key(entry, requested);
      const cached = previews.current.get(id);
      if (cached) return cached;

      // Native detail belongs to its caller. The active editor can save edits
      // before a foreground load without changing the grid entry's version.
      const share = requested !== 0 && entry.imgid !== activeEditorId;
      const pending = pendingPreviews.current;
      const existing = share ? pending.get(id) : undefined;
      if (existing) return existing;

      const request = (async () => {
        const kind = fileKind(entry.filename);
        const preferSource = requested > 0 && !entry.edited && (kind === "RAW" || kind === "JPEG");
        const cameraPreview = cameraPreviewFor(entry, requested);
        const disk = await fetchCachedThumb(entry, requested, preferSource, cameraPreview);
        const bitmap = disk ?? await fetchThumb(entry.imgid, requested, entry, false, foreground, preferSource, cameraPreview);
        if (epoch !== previewEpoch.current) { bitmap.close(); return null; }
        if (requested === 0) return bitmap;
        // In the active editor case an independent load may already have
        // filled this key. Retire its bitmap before taking ownership.
        previews.current.get(id)?.close();
        previews.current.set(id, bitmap);
        while (previews.current.size > 8) {
          const oldest = previews.current.keys().next().value!;
          previews.current.get(oldest)!.close(); previews.current.delete(oldest);
        }
        return bitmap;
      })();
      if (!share) return request;
      pending.set(id, request);
      try {
        return await request;
      } finally {
        if (pending.get(id) === request) pending.delete(id);
      }
    };
    const show = async () => {
      const cached = previews.current.get(key(current, size));
      if (cached) draw(cached);
      else {
        // A foreground preview starts independently of the speculative queue.
        await prepare(current.imgid);
        if (stale) return;
        const bitmap = await load(current, size, true);
        if (bitmap) {
          if (stale) { if (size === 0) bitmap.close(); } else { draw(bitmap); if (size === 0) bitmap.close(); }
        }
      }
      if (stale) return;
      setDetail(detailed); setBusy(false);
      if (detailed) return;
      const { count, both } = loadReadAhead();
      const neighbors = previewNeighbors(images, current.imgid, direction.current, count, both);
      if (!neighbors.length) return;
      const next = images[images.indexOf(current) + direction.current];
      // Give the next edited photo the full viewing time to render, once
      // the current Fit preview is ready. Keep the timer cancellable.
      timer = setTimeout(() => {
        for (const neighbor of neighbors) {
          void q.enqueue(async () => { await load(neighbor, size); }).catch(() => {});
        }
      }, next?.edited ? 0 : 180);
    };
    void show().catch(e => { if (!stale) { setError(String(e)); setBusy(false); } });
    return () => { stale = true; clearTimeout(timer); q.replace(); };
  }, [current?.imgid, current?.path, current?.version, current?.edited, activeEditorId, detailed, cameraFit, retry]);
  useEffect(() => { setZoom(null); }, [photo]);
  useEffect(() => () => { queue.current.replace(); ++previewEpoch.current; pendingPreviews.current.clear(); for (const bitmap of previews.current.values()) bitmap.close(); previews.current.clear(); ++keywordGeneration.current; }, []);
  useEffect(() => {
    if (targets.length && lists.length === targets.length)
      document.querySelector<HTMLInputElement>("[data-grid-keywords-dialog] input")?.focus();
  }, [targets, lists.length]);
  const change = async (add: string[], remove: string[]) => {
    if (writing) return;
    setWriting(true); setError("");
    try {
      const result = await Promise.allSettled(targets.map(async i => {
        const list = await commands.keywordsSet(i.imgid, [...new Set(add.map(parseKeyword).filter(Boolean))], remove);
        onKeywords(i.imgid, list); return list;
      }));
      setLists(result.map((r, n) => r.status === "fulfilled" ? r.value : lists[n] ?? []));
      if (result.some(r => r.status === "rejected")) setError(t("library.culling.updateError"));
    } finally { setWriting(false); }
  };
  const shared = lists.length === targets.length ? (lists[0] ?? []).filter(k => lists.every(l => l.includes(k))) : [];
  return <>
    {current && <div className="modal-backdrop culling-backdrop"
      onClick={e => { if (e.target === e.currentTarget) setPhoto(null); }} aria-modal="true">
      <CullLightbox dialogLabel={t("library.culling.preview")}
        head={<LightboxHead filename={current.filename} index={images.indexOf(current) + 1} total={images.length}
          onEdit={() => onEdit(current)} onKeywords={() => void keywords([current])} onClose={() => setPhoto(null)} context={scene?.context} />}
        stage={<>
        <div ref={viewport} style={{ flex: 1, minWidth: 0, overflow: detailed ? "auto" : "visible", minHeight: 0,
          cursor: detailed && spaceHeld ? (panning ? "grabbing" : "grab") : detailed ? "zoom-out" : "zoom-in" }}
          onClick={e => {
            if (panClick.current || space.current) { panClick.current = false; return; }
            if (e.detail === 1) {
              const image = canvas.current?.getBoundingClientRect();
              const box = e.currentTarget.getBoundingClientRect();
              if (image && image.width && image.height) zoomAnchor.current = {
                ix: Math.max(0, Math.min(1, (e.clientX - image.left) / image.width)),
                iy: Math.max(0, Math.min(1, (e.clientY - image.top) / image.height)),
                sx: e.clientX - box.left, sy: e.clientY - box.top,
              };
              setZoom(zoomRef.current === null ? 1 : null);
            }
          }}
          onDoubleClick={() => { if (!panClick.current && !space.current) onEdit(current); }}
          onPointerDown={e => {
            panClick.current = false;
            if (!detailed || !space.current || e.button !== 0) return;
            panClick.current = true; setPanning(true);
            const el = e.currentTarget; el.setPointerCapture(e.pointerId);
            const x = e.clientX, y = e.clientY, left = el.scrollLeft, top = el.scrollTop;
            const move = (event: PointerEvent) => { el.scrollLeft = left + x - event.clientX; el.scrollTop = top + y - event.clientY; };
            const end = () => { setPanning(false); el.removeEventListener("pointermove", move); el.removeEventListener("lostpointercapture", end); };
            el.addEventListener("pointermove", move); el.addEventListener("lostpointercapture", end);
          }}>
          <div style={{ minWidth: "100%", minHeight: "100%", width: "max-content", height: "max-content", display: "grid", placeItems: "center" }}>
            <div className="culling-photo" style={{ position: "relative", width: "max-content", height: "max-content" }}>
              <canvas ref={canvas} style={{ maxWidth: "none", flexShrink: 0, display: "block" }} />
              <CullOverlay critique={crit.critique} sel={sel} onSel={setSel} decision={decision}
                loading={crit.phase === "loading" && tab === "critique"} />
            </div>
          </div>
        </div>
        {error && <div role="alert" className="culling-photo-error">{error}<button type="button" className="cull-btn" onClick={() => setRetry(n => n + 1)}>{t("library.culling.retry")}</button></div>}
        <StageArrows hasPrevious={!!images[images.indexOf(current) - 1]} hasNext={!!images[images.indexOf(current) + 1]}
          onStep={dir => { const next = images[images.indexOf(current) + dir]; if (next) { direction.current = dir; setPhoto(next.imgid); } }} />
        </>}
        side={<CullSide image={current} tab={tab} onTab={setTabChosen} crit={crit} sel={sel} onSel={setSel}
          onChanges={onCullChanges} onOpenPrefs={openPrefs} scenePanel={scene?.panel(current)} />}
        foot={<>
        {scene?.strip(current, id => { direction.current = 1; setPhoto(id); })}
        <div className="culling-bottom-bar">
          <div className="culling-rating-controls">
            <span className="tool-group" role="group" aria-label={t("library.culling.rating")}>
              <button className={`tool${current.rejected ? " on" : ""}`} aria-label={t("library.culling.reject")} aria-pressed={current.rejected}
                title={t("library.culling.rejectShortcut")} onClick={() => onMeta(current, { rejected: !current.rejected })}><BanIcon /></button>
              {[1, 2, 3, 4, 5].map(n => <button key={n} className="tool"
                aria-label={t("library.culling.stars", { count: n })} aria-pressed={!current.rejected && current.rating >= n}
                title={t("library.culling.starsClear", { count: n })}
                onClick={() => onMeta(current, { rating: current.rating === n ? 0 : n, rejected: false })}>
                <StarIcon filled={!current.rejected && current.rating >= n} />
              </button>)}
            </span>
            <span className="tool-group" role="group" aria-label={t("library.culling.labels")}>
              {LABEL_COLORS.map((color, n) => {
                const name = t(["library.culling.red", "library.culling.yellow", "library.culling.green", "library.culling.blue", "library.culling.purple"][n]);
                return <button key={color} className={`swatch${current.labels & (1 << n) ? " on" : ""}`}
                  style={{ background: color }} aria-label={t("library.culling.colorLabel", { color: name })} aria-pressed={!!(current.labels & (1 << n))}
                  title={t(n < 4 ? "library.culling.colorLabelShortcut" : "library.culling.colorLabel", { color: name, key: n + 6 })}
                  onClick={() => onMeta(current, { labels: current.labels ^ (1 << n) })} />;
              })}
            </span>
          </div>
          <span className="culling-status" role="status">{busy ? t("library.culling.loading") : ""}</span>
          <div className="culling-view-controls">
            <button type="button" className="tool" aria-pressed={cameraPreviews} aria-label={t("library.culling.cameraPreviews")}
              title={t("library.culling.cameraPreviewsHint")}
              onClick={() => setCameraPreviews(value => !value)}><Camera size={16} aria-hidden="true" /></button>
            <ZoomBar disabled={false} register={show => {
              zoomShow.current = show;
              show(zoomRef.current ?? fitScale(), zoomRef.current === null);
            }} fit={fitScale} onZoom={value => {
              zoomAnchor.current = null;
              setZoom(typeof value === "number" ? value : null);
            }} onToggle={() => {
              zoomAnchor.current = null;
              setZoom(zoomRef.current === null ? 1 : null);
            }} />
          </div>
        </div></>} />
    </div>}
    {!!targets.length && <div className="modal-backdrop" data-grid-keywords-dialog role="dialog"
      aria-modal="true" aria-label={t("library.culling.gridKeywords")}>
      <div className="modal">
        <div className="modal-head">
          <h2>{t("library.culling.keywordPhotos", { n: targets.length })}</h2>
          <button type="button" className="tool" disabled={writing} aria-label={t("library.culling.close")}
            title={t("library.culling.close")} onClick={() => { ++keywordGeneration.current; setTargets([]); }}>
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="modal-body">
          <fieldset className="keyword-dialog-fields" disabled={writing}>
            {lists.length === targets.length && <KeywordingPanel keywords={shared} known={known}
              onAdd={names => { if (!writing) void change(names, []); }}
              onRemove={name => { if (!writing) void change([], [name]); }} />}
          </fieldset>
          {error && <p role="alert">{error}</p>}
        </div>
      </div>
    </div>}
  </>;
}
