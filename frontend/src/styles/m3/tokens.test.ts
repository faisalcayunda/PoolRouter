import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const root = process.cwd();

describe('M3 UI v2 typeface tokens', () => {
  it('loads Inter and JetBrains Mono locally and exposes font-code', () => {
    const tokens = readFileSync(`${root}/src/styles/m3/tokens.css`, 'utf8');
    const tailwind = readFileSync(`${root}/src/styles/m3/tailwind.css`, 'utf8');
    const packageJson = readFileSync(`${root}/package.json`, 'utf8');

    expect(tokens).toContain("--md-ref-typeface-brand: 'Inter Variable'");
    expect(tokens).toContain("--md-ref-typeface-plain: 'Inter Variable'");
    expect(tokens).toContain("--md-ref-typeface-code: 'JetBrains Mono'");
    expect(tailwind).toContain("@import '@fontsource-variable/inter'");
    expect(tailwind).toContain("@import '@fontsource/jetbrains-mono/400.css'");
    expect(tailwind).toContain('--font-code: var(--md-ref-typeface-code)');
    expect(packageJson).not.toContain('@fontsource-variable/roboto');
  });
});
