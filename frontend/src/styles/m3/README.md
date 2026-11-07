# M3 tokens (UI v2)

Single token source for the Web UI (ADR-0003 and ADR-0008 §2.2–§2.3). Components read `var(--md-sys-*)` only and never contain hex literals; ESLint (`no-restricted-syntax`) enforces that outside this folder.

| File           | Content                                                                                                                                                                                                                                                                                 |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `color.css`    | Light scheme, standard contrast, `--md-sys-color-*` including the extended roles `success`, `warning`, `info` (`on-*`, `*-container`, `on-*-container`)                                                                                                                                 |
| `tokens.css`   | Typescale, shape, elevation, state-layer, focus-indicator and motion tokens; Inter and JetBrains Mono reference typefaces                                                                                                                                                               |
| `theme.json`   | Seed `#2F55D4` and custom colours (reference copy, not read at runtime)                                                                                                                                                                                                                 |
| `tailwind.css` | Tailwind 4 `@theme inline` mapping: colour roles, `rounded-*`, `shadow-level*`, window size class breakpoints (`medium` 600, `expanded` 840, `large` 1200, `xlarge` 1600), `typescale-*`, and `font-code` utilities. Tailwind default colour, radius and breakpoint scales are disabled |

## Status

`theme.json` is the ARC-11 seed export. `color.css` maps its generated scheme onto the product's canonical token names; component code reads only those names.

## Updating the palette

1. Replace `color.css` and `theme.json` together.
2. Map exported names onto the canonical token names in `color.css`. Component code does not change. The extended roles remain `--md-sys-color-{success,warning,info}`, `on-*`, `*-container`, `on-*-container`.
3. If any hex value changes, recompute the contrast table first.

The code typeface is exposed as `font-code`; use it instead of Tailwind's default `font-mono`.

## Name mapping

Canonical `md.sys.color.on-primary-container` becomes `--md-sys-color-on-primary-container` in CSS and `bg-primary-container` / `text-on-primary-container` in Tailwind (role alias). No renaming was needed for the temporary files.
