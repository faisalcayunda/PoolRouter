import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const root = process.cwd();

describe('M3 UI v3 typeface tokens', () => {
  it('loads Hanken Grotesk, Inter and JetBrains Mono from npm and exposes font-code', () => {
    const tokens = readFileSync(`${root}/src/styles/m3/tokens.css`, 'utf8');
    const tailwind = readFileSync(`${root}/src/styles/m3/tailwind.css`, 'utf8');
    const packageJson = readFileSync(`${root}/package.json`, 'utf8');

    expect(tokens).toContain("--md-ref-typeface-brand: 'Hanken Grotesk Variable'");
    expect(tokens).toContain("--md-ref-typeface-plain: 'Inter Variable'");
    expect(tokens).toContain("--md-ref-typeface-code: 'JetBrains Mono'");
    expect(tailwind).not.toContain('@font-face');
    expect(tailwind).toContain("@import '@fontsource/jetbrains-mono/400.css'");
    expect(tailwind).toContain('--font-code: var(--md-ref-typeface-code)');
    expect(packageJson).not.toContain('@fontsource-variable/roboto');
  });
});
