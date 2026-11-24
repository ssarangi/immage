// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// People as the app mounts it: PeopleView with the faces model pack from the backend (the same pack, states
// and download consent as Preferences > AI models), so the first-run card can install it in place.
import { useState } from "react";
import { commands } from "./commands";
import type { ActionId } from "./aiPacks";
import { usePacks } from "./aiPacks";
import { useCatalog } from "./catalogStore";
import { PackConsentDialog } from "./PackDialogs";
import { PeopleView } from "./PeopleView";
import { t } from "./i18n";

export function PeoplePane({ folder }: { folder: string | null }) {
  const hasCatalog = useCatalog().sources.length > 0;
  const { packs, refresh, setPacks } = usePacks();
  const pack = packs?.find((p) => p.id === "faces") ?? null;
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const run = async (job: () => Promise<unknown>) => {
    setBusy(true);
    try { await job(); } catch { /* the pack's own state line shows the failure */ } finally { setBusy(false); void refresh(); }
  };
  const onPack = (a: ActionId) => {
    if (!pack) return;
    if (a === "download") setConsent(true);
    else if (a === "cancel") void commands.packCancel("faces").catch(() => {});
    else if (a === "resume" || a === "repair") void run(async () => { setPacks((l) => l && l.map((p) => (p.id === "faces" ? { ...p, state: "downloading", error: null } : p))); await commands.packDownload("faces", false); });
    else if (a === "check") void run(() => commands.packVerify("faces"));
    else if (a === "removePartial") void run(() => commands.packRemove("faces", true, true));
  };
  return (
    <>
      <PeopleView folder={folder} hasCatalog={hasCatalog} pack={pack} packBusy={busy} onPack={onPack} />
      {consent && pack && (
        <PackConsentDialog info={pack} name={t("aimodels.faces.name")} what={t("aimodels.faces.what", { n: 2 })}
                           license={() => commands.packLicense("faces")}
                           onConfirm={() => { setConsent(false); void run(() => commands.packDownload("faces", true)); }} onClose={() => setConsent(false)} />
      )}
    </>
  );
}
