# M3 tokens (temporary, DV-2)

Single token source for the Web UI (ADR-0003 rev 4 sections 3.5, 7.2, 7.3). Components read `var(--md-sys-*)` only and never contain hex literals; ESLint (`no-restricted-syntax`) enforces that outside this folder.

| File | Content |
|---|---|
| `color.css` | Light scheme, standard contrast, `--md-sys-color-*` including the extended roles `success`, `warning`, `info` (`on-*`, `*-container`, `on-*-container`) |
| `tokens.css` | Typescale, shape, elevation, state-layer, focus-indicator and motion tokens |
| `theme.json` | Seed `#5C6670` and custom colours (reference copy, not read at runtime) |
| `tailwind.css` | Tailwind 4 `@theme inline` mapping: colour roles, `rounded-*`, `shadow-level*`, window size class breakpoints (`medium` 600, `expanded` 840, `large` 1200, `xlarge` 1600) and `typescale-*` utilities. Tailwind default colour, radius and breakpoint scales are disabled |

## Status

`color.css`, `tokens.css`, and `theme.json` come from Jude's temporary package (SHAA-168 attachments). They were derived programmatically with `@material/material-color-utilities` 0.3.0 and are **not** a Material Theme Builder export. The real export arrives in SHAA-187.

## Replacing the export (SHAA-187)

1. Replace `color.css` and `theme.json` in one commit that touches only this folder.
2. Map the export's names onto the names above inside `color.css`. Component code does not change. The extended role names are locked: `--md-sys-color-{success,warning,info}`, `on-*`, `*-container`, `on-*-container`.
3. If any hex value changes, Jude recomputes the contrast table first.

## Name mapping

Canonical `md.sys.color.on-primary-container` becomes `--md-sys-color-on-primary-container` in CSS and `bg-primary-container` / `text-on-primary-container` in Tailwind (role alias). No renaming was needed for the temporary files.
