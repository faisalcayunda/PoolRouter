import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const root = process.cwd();

describe('M3 UI v2 typeface tokens', () => {
  it('bundles Open Sans locally, loads JetBrains Mono and exposes font-code', () => {
    const tokens = readFileSync(`${root}/src/styles/m3/tokens.css`, 'utf8');
    const tailwind = readFileSync(`${root}/src/styles/m3/tailwind.css`, 'utf8');
    const packageJson = readFileSync(`${root}/package.json`, 'utf8');

    expect(tokens).toContain("--md-ref-typeface-brand: 'Open Sans'");
    expect(tokens).toContain("--md-ref-typeface-plain: 'Open Sans'");
    expect(tokens).toContain("--md-ref-typeface-code: 'JetBrains Mono'");
    expect(tailwind).toContain("font-family: 'Open Sans'");
    expect(tailwind).toContain("url('../../assets/fonts/OpenSans-latin.woff2')");
    expect(tailwind).toContain("@import '@fontsource/jetbrains-mono/400.css'");
    expect(tailwind).toContain('--font-code: var(--md-ref-typeface-code)');
    expect(packageJson).not.toContain('@fontsource-variable/roboto');
  });
});
