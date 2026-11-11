import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  FIGMA_COLOR_ROLES,
  FIGMA_ELEVATION_LEVEL1,
  FIGMA_SHAPE,
  FIGMA_TYPESCALE,
} from './figmaSpec';

/**
 * UI v3 (ARC-366, spec ui-v3-figma §3): nilai token di kode sama dengan spec. Dibaca dari berkas,
 * karena jsdom tidak memuat CSS; `tailwind.css` membuat utilitas dari variabel yang sama.
 */
const root = process.cwd();
const read = (path: string) => readFileSync(`${root}/${path}`, 'utf8');

function cssVars(css: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const m of css.matchAll(/(--[a-z0-9-]+):\s*([^;]+);/g)) {
    if (!out.has(m[1]!)) out.set(m[1]!, m[2]!.trim());
  }
  return out;
}

const color = cssVars(read('src/styles/m3/color.css'));
const tokens = cssVars(read('src/styles/m3/tokens.css'));
const tailwind = read('src/styles/m3/tailwind.css');
const theme = JSON.parse(read('src/styles/m3/theme.json')) as {
  schemes: { light: Record<string, string> };
};

const camel = (role: string) => role.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());

describe('UI v3 color roles (spec §3)', () => {
  it('defines exactly the 49 roles of the spec', () => {
    expect(Object.keys(FIGMA_COLOR_ROLES)).toHaveLength(49);
    const defined = [...color.keys()].filter((k) => k.startsWith('--md-sys-color-'));
    expect(defined.map((k) => k.replace('--md-sys-color-', '')).sort()).toEqual(
      Object.keys(FIGMA_COLOR_ROLES).sort(),
    );
  });

  it.each(Object.entries(FIGMA_COLOR_ROLES))('color.css %s = %s', (role, hex) => {
    expect(color.get(`--md-sys-color-${role}`)?.toLowerCase()).toBe(hex);
  });

  it.each(Object.entries(FIGMA_COLOR_ROLES))('theme.json %s = %s', (role, hex) => {
    expect(theme.schemes.light[camel(role)]?.toLowerCase()).toBe(hex);
  });

  it('maps every role to a Tailwind colour utility, including primary-text', () => {
    for (const role of Object.keys(FIGMA_COLOR_ROLES)) {
      expect(tailwind).toContain(`--color-${role}: var(--md-sys-color-${role});`);
    }
  });
});

describe('UI v3 typeface (spec §3)', () => {
  it('uses Hanken Grotesk for brand, Inter for plain and keeps JetBrains Mono', () => {
    expect(tokens.get('--md-ref-typeface-brand')).toMatch(/^'Hanken Grotesk Variable'/);
    expect(tokens.get('--md-ref-typeface-plain')).toMatch(/^'Inter Variable'/);
    expect(tokens.get('--md-ref-typeface-code')).toMatch(/^'JetBrains Mono'/);
  });

  it('bundles the fonts from npm (self-host, font-src self) and drops Open Sans', () => {
    const pkg = JSON.parse(read('package.json')) as { dependencies: Record<string, string> };
    expect(pkg.dependencies['@fontsource-variable/hanken-grotesk']).toMatch(/^\d+\.\d+\.\d+$/);
    expect(pkg.dependencies['@fontsource-variable/inter']).toBeDefined();
    expect(tailwind).toContain("@import '@fontsource-variable/hanken-grotesk/wght.css'");
    expect(tailwind).toContain("@import '@fontsource-variable/inter/wght.css'");
    expect(tailwind).not.toMatch(/Open Sans|OpenSans/);
    expect(read('src/styles/m3/tokens.css')).not.toMatch(/Open Sans/);
    expect(() => read('src/assets/fonts/OpenSans-latin.woff2')).toThrow();
  });
});

describe('UI v3 typescale, shape, elevation (spec §3)', () => {
  it.each(Object.entries(FIGMA_TYPESCALE))('%s', (name, v) => {
    const p = `--md-sys-typescale-${name}`;
    expect(tokens.get(`${p}-size`)).toBe(v.size);
    expect(tokens.get(`${p}-line-height`)).toBe(v.lineHeight);
    expect(tokens.get(`${p}-weight`)).toBe(v.weight);
    expect(tokens.get(`${p}-font`)).toBe(`var(--md-ref-typeface-${v.font})`);
  });

  it.each(Object.entries(FIGMA_SHAPE))('corner %s = %s', (name, value) => {
    expect(tokens.get(`--md-sys-shape-corner-${name}`)).toBe(value);
  });

  it('elevation level1 is Figma Shadow/xs and the Tailwind shadow uses it', () => {
    expect(tokens.get('--md-sys-elevation-level1')).toBe(FIGMA_ELEVATION_LEVEL1);
    expect(tailwind).toContain('--shadow-level1: var(--md-sys-elevation-level1);');
  });
});
