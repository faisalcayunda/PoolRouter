import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import svgr from 'vite-plugin-svgr';
import { cspPlugin } from './vite.csp';

/**
 * Role seam (SHAA-203 design review): `?peran=` may only pin a role in a build made for the e2e
 * suite. `define` replaces `VITE_TEST_ROLE_SEAM` with a literal, so in a production build the branch
 * in `src/app/routes.tsx` is constant-folded away and the query parameter can never set the role.
 * `make e2e` builds with `--mode playwright`; every other mode, including the default production
 * build, gets `false`. Vitest runs in `test` mode and is included so the seam can be tested from
 * both sides — the production build is covered by the e2e smoke against `vite preview --mode
 * playwright` plus the `PAGE` check in `e2e/shell.spec.ts`.
 */
export default defineConfig(({ mode }) => ({
  plugins: [react(), tailwindcss(), svgr({ include: '**/*.svg?react' }), cspPlugin()],
  define: {
    // `import.meta.env` keys are substituted like `process.env`; Vite folds the resulting literal
    // into the bundle, so the production build has no `?peran=` read at all.
    'import.meta.env.VITE_TEST_ROLE_SEAM': JSON.stringify(mode === 'playwright' || mode === 'test'),
  },
  server: { host: true, port: 5173 },
  preview: { host: true, port: 4173 },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./tests/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}', 'tests/**/*.test.{ts,tsx}'],
    // e2e/ is Playwright's (make e2e); Vitest must not pick those specs up.
    exclude: ['e2e/**', 'node_modules/**', 'dist/**'],
    css: false,
  },
}));
