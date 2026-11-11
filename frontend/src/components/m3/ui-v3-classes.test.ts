import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Perubahan wajib S3 dari spec ui-v3-figma §3 dan peta komponen §4 (ARC-366). Lime `primary`
 * hanya 1,97:1 di `surface`, jadi teks dan penanda seleksi memakai `primary-text`, dan fokus selalu
 * `secondary`. Dibaca dari sumber seperti `catalog.test.ts`: kelas Tailwind tidak punya efek di jsdom.
 */
const HERE = dirname(fileURLToPath(import.meta.url));
const src = (file: string) => readFileSync(join(HERE, file), 'utf8');

describe('lime is never text or a lone selection marker (spec §3)', () => {
  it('CircularProgress draws in primary-text', () => {
    expect(src('CircularProgress.tsx')).toContain('text-primary-text');
    expect(src('CircularProgress.tsx')).not.toMatch(/text-primary(?![-\w])/);
  });

  it('Radio dot and border use primary-text', () => {
    const s = src('Radio.tsx');
    expect(s).toContain('group-data-[selected]:border-primary-text');
    expect(s).toMatch(/rounded-full bg-primary-text opacity-0/);
  });

  it('Tabs active line is primary-text, active label on-surface', () => {
    const s = src('Tabs.tsx');
    expect(s).toContain('data-[selected]:after:bg-primary-text');
    expect(s).toContain('data-[selected]:text-on-surface');
  });

  it('Checkbox and Switch keep the lime fill but get a primary-text border', () => {
    const cb = src('Checkbox.tsx');
    expect(cb).toContain(
      'group-data-[selected]:border-primary-text group-data-[selected]:bg-primary',
    );
    expect(cb).toContain(
      'group-data-[indeterminate]:border-primary-text group-data-[indeterminate]:bg-primary',
    );
    expect(src('Switch.tsx')).toContain(
      'group-data-[selected]:border-primary-text group-data-[selected]:bg-primary',
    );
  });

  it('Switch thumb in the off position is on-surface-variant (5,27:1)', () => {
    const s = src('Switch.tsx');
    expect(s).toContain('rounded-full bg-on-surface-variant transition-transform');
    expect(s).not.toMatch(/bg-outline(?![-\w])/);
  });

  it('DatePicker and TimeField carets are primary-text', () => {
    for (const file of ['DatePicker.tsx', 'TimeField.tsx']) {
      expect(src(file), file).toContain('caret-primary-text');
      expect(src(file), file).not.toMatch(/caret-primary(?![-\w])/);
    }
  });
});

describe('LinearProgress standard (spec §3, Figma 2:21304)', () => {
  it('uses a surface-container track and an accent bar for both sizes', () => {
    const s = src('LinearProgress.tsx');
    expect(s).not.toContain('bg-secondary-container');
    expect(s).not.toMatch(/'bg-primary'/);
    expect(s).toContain("'h-1 bg-surface-container'");
    expect(s).toMatch(/'h-full',\s*'bg-accent'/);
  });
});

describe('warning border is solid (spec §3, ARC-369 AC1)', () => {
  it.each(['StatusLabel.tsx', 'InlineAlert.tsx'])('%s', (file) => {
    expect(src(file)).toMatch(/border border-warning bg-warning-container/);
    expect(src(file)).not.toMatch(/border-warning\/\d+/);
  });
});

describe('focus is always secondary (spec §3)', () => {
  it.each(['TextField.tsx', 'Select.tsx', 'ComboBox.tsx', 'DatePicker.tsx'])('%s', (file) => {
    const s = src(file);
    expect(s).toContain('ring-secondary');
    expect(s).not.toContain('ring-accent-container');
  });

  it('TimeField focus border is secondary', () => {
    const s = src('TimeField.tsx');
    expect(s).toContain('data-[focus-within]:border-secondary');
    expect(s).not.toMatch(/border-primary(?![-\w])/);
  });
});

describe('component map (spec §4)', () => {
  it('NavigationDrawer is 272 px with an outline-variant right border on both variants', () => {
    const s = src('NavigationDrawer.tsx');
    expect(s).toContain('w-[272px]');
    expect(s).not.toContain('w-60');
    expect(s.match(/border-r border-outline-variant/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it('Dialog title is headline-small and the surface uses corner extra-large', () => {
    const s = src('Dialog.tsx');
    expect(s).toContain('typescale-headline-small');
    expect(s).toContain('rounded-extra-large p-6');
  });

  it('Card container is the Figma card: radius 16 via corner-large, outline-variant, level1', () => {
    expect(src('Card.tsx')).toContain(
      "container: 'rounded-large border border-outline-variant bg-surface-container-lowest shadow-level1'",
    );
  });

  it('text fields are 40 px with corner extra-small (Figma Input 2:22114)', () => {
    for (const file of ['TextField.tsx', 'Select.tsx', 'ComboBox.tsx', 'DatePicker.tsx']) {
      expect(src(file), file).toMatch(/h-10[^']*rounded-extra-small|rounded-extra-small[^']*h-10/);
    }
  });

  it('TopAppBar carries no search field or notification bell', () => {
    const s = src('TopAppBar.tsx');
    expect(s).not.toMatch(/search|notification|bell/i);
  });
});
