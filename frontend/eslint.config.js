import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import jsxA11y from 'eslint-plugin-jsx-a11y';
import reactHooks from 'eslint-plugin-react-hooks';

export default tseslint.config(
  {
    ignores: [
      'dist',
      'coverage',
      'node_modules',
      'src/api/schema.gen.ts',
      'src/i18n/i18next.d.ts',
      '.cache',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  jsxA11y.flatConfigs.recommended,
  reactHooks.configs.flat.recommended,
  {
    languageOptions: { globals: { ...globals.browser } },
    rules: {
      // Hex literals belong in src/styles/m3/ only (SHAA-197 AC: one token source).
      'no-restricted-syntax': [
        'error',
        {
          selector: 'Literal[value=/#[0-9a-fA-F]{3,8}\\b/]',
          message: 'No hex colour literals outside src/styles/m3/; use var(--md-sys-color-*).',
        },
      ],
    },
  },
  { files: ['*.config.{js,ts}', 'scripts/**'], languageOptions: { globals: { ...globals.node } } },
  // Playwright specs: `test`/`expect` come from @playwright/test, not Vitest.
  {
    files: ['e2e/**/*.ts'],
    languageOptions: { globals: { ...globals.node } },
    rules: {
      // Playwright fixtures take a callback parameter literally named `use`;
      // it is not React's `use`, so the hook rules do not apply here.
      'react-hooks/rules-of-hooks': 'off',
      'react-hooks/exhaustive-deps': 'off',
    },
  },
);
