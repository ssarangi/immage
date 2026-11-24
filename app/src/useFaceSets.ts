// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the React side of facesFilter.ts: a number that changes when a path set arrives or is dropped
import { useSyncExternalStore } from "react";
import { faceSetsVersion, subscribeFaceSets } from "./facesFilter";

export const useFaceSetsVersion = () => useSyncExternalStore(subscribeFaceSets, faceSetsVersion);
