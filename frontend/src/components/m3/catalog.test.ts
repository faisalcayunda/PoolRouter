import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Katalog §7.1 coverage (SHAA-197). The table in `docs/adr/0003-stack-web-ui.md`
 * is the source of truth; this test fails when a catalog name has no exported
 * component in `src/components/m3/`, or when something sits in that folder that
 * the catalog does not name.
 *
 * The check reads source text rather than importing the modules: a glob import
 * would eagerly evaluate every component (and every test) just to count names.
 */
// Resolved relative to this file, so it holds both on the host and in the test
// container (which mounts the repo root at /work with cwd /work/frontend).
const HERE = dirname(fileURLToPath(import.meta.url));
const ADR = join(HERE, '../../../../docs/adr/0003-stack-web-ui.md');
const COMPONENT_DIR = HERE;

/**
 * Parses the §7.1 tables. The component table carries the catalog name in the
 * third column; the "Ekstensi lokal" table names it in the first.
 */
function catalogNames(): string[] {
  if (!existsSync(ADR)) {
    throw new Error(
      `ADR-0003 tidak ditemukan di ${ADR}; jalankan lewat \`make -C frontend test\`.`,
    );
  }
  const text = readFileSync(ADR, 'utf8');
  const section = text.split('### 7.1')[1]?.split('### 7.2')[0];
  if (!section) throw new Error('ADR-0003: section 7.1 not found');
  const names = new Set<string>();
  let column = 2;
  for (const line of section.split('\n')) {
    if (line.startsWith('**Ekstensi lokal')) {
      column = 0;
      continue;
    }
    if (!line.startsWith('|')) continue;
    const cell = (line.split('|').map((c) => c.trim())[column + 1] ?? '').replace(/\\\|/g, '|');
    for (const match of cell.matchAll(/`([A-Z][A-Za-z]+)`/g)) names.add(match[1]!);
  }
  return [...names].sort();
}

/** PascalCase `export function` names per component file, excluding tests and helpers. */
function exportedNames(): Map<string, string> {
  const out = new Map<string, string>();
  for (const file of readdirSync(COMPONENT_DIR)) {
    if (!file.endsWith('.tsx') || file.endsWith('.test.tsx')) continue;
    const source = readFileSync(join(COMPONENT_DIR, file), 'utf8');
    for (const match of source.matchAll(/^export function ([A-Z][A-Za-z]*)/gm)) {
      out.set(match[1]!, file);
    }
  }
  return out;
}

describe('katalog §7.1', () => {
  it('parses the catalog names out of the ADR', () => {
    const names = catalogNames();
    // 23 components + 6 extensions (SHAA-197): a guard against a silently empty parse.
    expect(names.length).toBeGreaterThanOrEqual(29);
    for (const required of [
      'NavigationDrawer',
      'TopAppBar',
      'TimeField',
      'LinearProgress',
      'CircularProgress',
      'StatusLabel',
      'InlineAlert',
      'Breadcrumbs',
    ]) {
      expect(names).toContain(required);
    }
  });

  it('exports a component for every catalog name', () => {
    const exported = exportedNames();
    const missing = catalogNames().filter((name) => !exported.has(name));
    expect(missing).toEqual([]);
  });

  it('keeps every M3 component file anchored on a catalog name or an extension', () => {
    // The ADR allows compositions ("komponen di tabel ini atau komposisinya"), so
    // sub-parts such as `MenuItem` or `TabPanel` are legitimate exports. What must
    // not happen is a component file that implements nothing in the catalog.
    const catalog = catalogNames();
    const files = readdirSync(COMPONENT_DIR).filter(
      (file) => file.endsWith('.tsx') && !file.endsWith('.test.tsx'),
    );
    const unanchored: string[] = [];
    for (const file of files) {
      const source = readFileSync(join(COMPONENT_DIR, file), 'utf8');
      const exports = [...source.matchAll(/^export function ([A-Z][A-Za-z]*)/gm)].map((m) => m[1]!);
      if (exports.length > 0 && !exports.some((name) => catalog.includes(name))) {
        unanchored.push(file);
      }
    }
    expect(unanchored).toEqual([]);
  });
});
