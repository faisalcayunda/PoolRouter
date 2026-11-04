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
  server: {
    host: true,
    port: 5173,
    // Admin API on the same origin as the UI (ADR-0003 section 3.9): in dev the Vite server proxies
    // the prefix, as nginx does in the image. `changeOrigin: false` keeps the browser's `Host` and
    // `Origin` intact so the backend's CSRF and Origin checks see the real values. The target is
    // non-secret and can be overridden with POOLROUTER_ADMIN_API_PROXY_TARGET.
    proxy: {
      '/admin/v1': {
        target: process.env.POOLROUTER_ADMIN_API_PROXY_TARGET ?? 'http://127.0.0.1:8082',
        changeOrigin: false,
      },
    },
  },
  preview: { host: true, port: 4173 },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./tests/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}', 'tests/**/*.test.{ts,tsx}'],
    // e2e/ is Playwright's (make e2e); Vitest must not pick those specs up.
    exclude: ['e2e/**', 'node_modules/**', 'dist/**'],
    css: false,
    // SHAA-406: gate runs beside other lanes' builds; jsdom + React 19 wall-clock tests need headroom.
    // Default 5 s is insufficient under host CPU contention (load average ~4 on 4 cores).
    // Worst case measured: 3.4 s per file quiet / ~5 s at incident load; 30 s gives 6x margin
    // for the 16 workers on 4 cores while other lanes build.
    testTimeout: 30_000,
  },
}));
