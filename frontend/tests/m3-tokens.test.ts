import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** ADR-0008 sections 2.2-2.3: seed #2F55D4, Inter, JetBrains Mono. ARC-16 / AC-7. */
const HERE = dirname(fileURLToPath(import.meta.url));
const FRONTEND = join(HERE, '..');
const read = (path: string) => readFileSync(join(FRONTEND, path), 'utf8');

const theme = JSON.parse(read('src/styles/m3/theme.json')) as {
  seed: string;
  description: string;
  schemes: { light: Record<string, string> };
};
const colorCss = read('src/styles/m3/color.css');
const tokensCss = read('src/styles/m3/tokens.css');
const tailwindCss = read('src/styles/m3/tailwind.css');

const kebab = (role: string) => role.replace(/([A-Z])/g, '-$1').toLowerCase();

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    return statSync(path).isDirectory() ? sourceFiles(path) : [path];
  });
}

describe('M3 colour export from ARC-11', () => {
  it('uses the seed and derivation recorded in ARC-11', () => {
    expect(theme.seed).toBe('#2F55D4'); // eslint-disable-line no-restricted-syntax -- seed under test (AC-7)
    expect(theme.description).toContain('0.3.0');
  });

  it.each(Object.entries(theme.schemes.light))('maps %s to a CSS token', (role, value) => {
    expect(colorCss).toContain(`--md-sys-color-${kebab(role)}: ${value};`);
  });
});

describe('typefaces', () => {
  it('sets brand and plain to Inter, and a new code token to JetBrains Mono', () => {
    expect(tokensCss).toContain(
      "--md-ref-typeface-brand: 'Inter Variable', system-ui, sans-serif;",
    );
    expect(tokensCss).toContain(
      "--md-ref-typeface-plain: 'Inter Variable', system-ui, sans-serif;",
    );
    expect(tokensCss).toContain("--md-ref-typeface-code: 'JetBrains Mono', ui-monospace, monospace;");
  });

  it('exposes a font-code Tailwind utility', () => {
    expect(tailwindCss).toContain('--font-code: var(--md-ref-typeface-code);');
  });

  it('pins the font packages and drops Roboto', () => {
    const pkg = JSON.parse(read('package.json')) as { dependencies: Record<string, string> };
    expect(pkg.dependencies['@fontsource-variable/roboto']).toBeUndefined();
    expect(pkg.dependencies['@fontsource-variable/inter']).toBe('5.3.0');
    expect(pkg.dependencies['@fontsource/jetbrains-mono']).toBe('5.3.0');
  });

  it('leaves no font-mono class in src', () => {
    const offenders = sourceFiles(join(FRONTEND, 'src')).filter((file) =>
      readFileSync(file, 'utf8').includes('font-mono'),
    );
    expect(offenders).toEqual([]);
  });

  it('uses font-code on the audit user id and the payload pre', () => {
    expect(read('src/features/audit-akses/AuditScreen.tsx')).toContain('font-code');
    expect(read('src/features/log/PayloadDialog.tsx')).toContain('<pre className="font-code');
  });
});
