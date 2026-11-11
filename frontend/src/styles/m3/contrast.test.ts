import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CONTRAST_CLAIMS, CONTRAST_PAIRS } from './figmaSpec';

/**
 * Tabel kontras spec ui-v3-figma §3.1 (WCAG 2.2: teks >= 4,5:1, kontrol dan fokus >= 3:1), dihitung
 * dari nilai di `color.css`, bukan dari salinan spec: mengubah token tanpa memeriksa pasangan
 * gagal di sini.
 */
const css = readFileSync(`${process.cwd()}/src/styles/m3/color.css`, 'utf8');
const role = (name: string): string => {
  const m = new RegExp(`--md-sys-color-${name}:\\s*(#[0-9a-fA-F]{6});`).exec(css);
  if (!m) throw new Error(`role ${name} tidak ada di color.css`);
  return m[1]!;
};

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function ratio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

describe('kontras pasangan spec §3.1', () => {
  it.each(CONTRAST_PAIRS)('%s di %s >= %s', (fg, bg, min) => {
    expect(ratio(role(fg), role(bg))).toBeGreaterThanOrEqual(min);
  });

  it.each(Object.entries(CONTRAST_CLAIMS))('%s sama dengan rasio di spec (%s)', (pair, claim) => {
    const [fg, bg] = pair.split('/') as [string, string];
    expect(ratio(role(fg), role(bg))).toBeCloseTo(claim, 1);
  });

  it('every claimed ratio is a tested pair, and every pair has a claim', () => {
    const pairs = new Set(CONTRAST_PAIRS.map(([fg, bg]) => `${fg}/${bg}`));
    expect([...pairs].sort()).toEqual(Object.keys(CONTRAST_CLAIMS).sort());
  });

  it('lime primary on surface fails 3:1, so it is only a fill behind on-primary', () => {
    expect(ratio(role('primary'), role('surface'))).toBeLessThan(3);
  });
});
