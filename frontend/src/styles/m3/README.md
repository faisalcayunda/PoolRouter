# M3 tokens (UI v2)

Single token source for the Web UI (ADR-0003 and ADR-0008 §2.2–§2.3). Components read `var(--md-sys-*)` only and never contain hex literals; ESLint (`no-restricted-syntax`) enforces that outside this folder.

| File           | Content                                                                                                                                                                                                                                                                                 |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `color.css`    | Light scheme, standard contrast, `--md-sys-color-*` including the extended roles `success`, `warning`, `info` (`on-*`, `*-container`, `on-*-container`)                                                                                                                                 |
| `tokens.css`   | Typescale, shape, elevation, state-layer, focus-indicator and motion tokens; Hanken Grotesk, Inter and JetBrains Mono reference typefaces                                                                                                                                               |
| `theme.json`   | Seed `#2F55D4` and custom colours (reference copy, not read at runtime)                                                                                                                                                                                                                 |
| `tailwind.css` | Tailwind 4 `@theme inline` mapping: colour roles, `rounded-*`, `shadow-level*`, window size class breakpoints (`medium` 600, `expanded` 840, `large` 1200, `xlarge` 1600), `typescale-*`, and `font-code` utilities. Tailwind default colour, radius and breakpoint scales are disabled |

## Status

ARC-366 (UI v3) replaced the ARC-261 slate/sky palette with the Figma "Admin Dashboard" values from spec `products/poolrouter/specs/ui-v3-figma/spec.md` §3 (KB). `color.css` is hand-written; `theme.json` is a reference copy of the same values, and `figma-tokens.test.ts` checks both against `figmaSpec.ts`. Component code reads only the canonical token names. Extra roles: `outline-soft` (control border), `primary-text` (lime text and selection markers, 5.93:1; lime `primary` is a fill behind `on-primary` only, 1.97:1 on white), `accent` (progress bar) and `accent-container` (selected chip and avatar fill). `contrast.test.ts` checks every pair of spec §3.1.

Hanken Grotesk (brand) and Inter (plain) come from `@fontsource-variable/hanken-grotesk` and `@fontsource-variable/inter` (SIL OFL 1.1, `src/assets/fonts/OFL.txt`); Vite bundles the woff2 files, so they are served from the app origin (`font-src 'self'`).

## Updating the palette

1. Replace `color.css` and `theme.json` together.
2. Map exported names onto the canonical token names in `color.css`. Component code does not change. The extended roles remain `--md-sys-color-{success,warning,info}`, `on-*`, `*-container`, `on-*-container`.
3. If any hex value changes, recompute the contrast table first.

The code typeface is exposed as `font-code`; use it instead of Tailwind's default `font-mono`.

## Name mapping

Canonical `md.sys.color.on-primary-container` becomes `--md-sys-color-on-primary-container` in CSS and `bg-primary-container` / `text-on-primary-container` in Tailwind (role alias). No renaming was needed for the temporary files.
