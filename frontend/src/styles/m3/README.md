# M3 tokens

Single token source for the Web UI (ADR-0008 section 2.2, replacing ADR-0003 sections 3.5, 7.2, 7.3). Components read `var(--md-sys-*)` only and never contain hex literals; ESLint (`no-restricted-syntax`) enforces that outside this folder.

| File           | Content                                                                                                                                                                                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `color.css`    | Light scheme, standard contrast, `--md-sys-color-*` including the extended roles `success`, `warning`, `info` (`on-*`, `*-container`, `on-*-container`)                                                                                                                   |
| `tokens.css`   | Typeface, typescale, shape, elevation, state-layer, focus-indicator and motion tokens                                                                                                                                                                                     |
| `theme.json`   | Seed `#2F55D4` and custom colours (reference copy, not read at runtime)                                                                                                                                                                                                   |
| `tailwind.css` | Tailwind 4 `@theme inline` mapping: colour roles, `font-sans`/`font-brand`/`font-code`, `rounded-*`, `shadow-level*`, window size class breakpoints (`medium` 600, `expanded` 840, `large` 1200, `xlarge` 1600) and `typescale-*` utilities. Tailwind default colour, radius and breakpoint scales are disabled |

## Provenance

`theme.json` and `color.css` are copied from the ARC-11 Penpot theme export (seed `#2F55D4`, generated 5 November 2026). Method: `@material/material-color-utilities` **0.3.0**, `SchemeFidelity` (Material Theme Builder "Color match" equivalent), light scheme, standard contrast, extended colours `success`/`warning`/`info` with harmonize off. `color.css` was generated from `theme.json` by name mapping only — no hand-edited values. The contrast table for the 45 roles is in ARC-11's `out-contrast.csv`.

## Typefaces (ADR-0008 section 2.3)

Inter and JetBrains Mono are self-hosted through `@fontsource-variable/inter` and `@fontsource/jetbrains-mono` (both pinned 5.3.0) and imported in `src/main.tsx`.

- `--md-ref-typeface-brand` and `--md-ref-typeface-plain`: `'Inter Variable', system-ui, sans-serif`.
- `--md-ref-typeface-code`: `'JetBrains Mono', ui-monospace, monospace`. Use the `font-code` utility for transaction ids, `queue_id`, `client_id`, one-time secrets, masked NIK, and JSON payloads.
- Neither font is fetched from a third party: CSP stays `font-src 'self'`ADR-0003 section 3.9).

## Updating the export

1. Replace `color.css` and `theme.json` in one commit that touches only this folder.
2. Map the export's names onto the names above inside `color.css`. Component code does not change. The extended role names are locked: `--md-sys-color-{success,warning,info}`, `on-*`, `*-container`, `on-*-container`.
3. Recompute the contrast table and attach it to the design ticket before committing.

## Name mapping

Canonical `md.sys.color.onPrimaryContainer` becomes `--md-sys-color-on-primary-container` in CSS and `bg-primary-container` / `text-on-primary-container` in Tailwind (role alias). No renaming beyond kebab case was needed.
