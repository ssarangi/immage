// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

export function Seg<T extends string>({ value, options, onChange, label, disabled }: {
  value: T; options: { id: T; label: string; disabled?: boolean }[]; onChange(v: T): void; label: string; disabled?: boolean;
}) {
  return (
    <div className="st-seg" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.id} type="button" aria-pressed={value === o.id} disabled={disabled || o.disabled} onClick={() => onChange(o.id)}>{o.label}</button>
      ))}
    </div>
  );
}
