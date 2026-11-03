import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import svgr from 'vite-plugin-svgr';
import { cspPlugin } from './vite.csp';

export default defineConfig({
  plugins: [react(), tailwindcss(), svgr({ include: '**/*.svg?react' }), cspPlugin()],
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
});
