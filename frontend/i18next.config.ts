import { defineConfig } from 'i18next-cli';
import ignore from './src/i18n/status-ignore-keys.json' with { type: 'json' };

export default defineConfig({
  locales: ['id', 'en'],
  extract: {
    input: ['src/**/*.{ts,tsx}'],
    ignore: ['src/**/*.test.{ts,tsx}', 'src/api/schema.gen.ts'],
    output: 'src/i18n/messages/{{language}}.json',
    primaryLanguage: 'id',
    defaultNS: 'translation',
    keySeparator: '.',
    nsSeparator: false,
  },
  types: {
    input: ['src/i18n/messages/id.json'],
    output: 'src/i18n/i18next.d.ts',
    resourcesFile: 'src/i18n/resources.d.ts',
  },
  status: { checkIdentical: true, ignoreKeys: ignore.status.ignoreKeys },
} as never);
