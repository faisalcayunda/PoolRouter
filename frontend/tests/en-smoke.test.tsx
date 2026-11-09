import { beforeAll, describe, expect, it, vi } from 'vitest';
import { screen } from '@testing-library/react';
import type { i18n } from 'i18next';
import type { ApiClient } from '../src/api/client';
import { AuditScreen } from '../src/features/audit-akses/AuditScreen';
import { NikSearchScreen } from '../src/features/auditor/NikSearchScreen';
import { LogPageScreen } from '../src/features/log/LogPageScreen';
import { ProviderFormScreen } from '../src/features/provider/ProviderFormScreen';
import { ProviderListScreen } from '../src/features/provider/ProviderListScreen';
import { QuotaScreen } from '../src/features/quota/QuotaScreen';
import { SecretScreen } from '../src/features/secret/SecretScreen';
import { createI18n, loadLanguage } from '../src/i18n/i18n';
import { renderWithI18n } from './render';

/**
 * `en` smoke test per screen (SHAA-203 acceptance criteria).
 *
 * The catalogue is the source of the copy, so every screen in scope must render in `en` with no
 * missing key. The instance runs in test mode (`throwOnMissingKey`): a screen that invents a key
 * or leans on the `id` fallback fails here instead of quietly rendering Indonesian in an English
 * UI. There is deliberately no `onError` handler — a missing key must abort the render, not be
 * swallowed.
 */
const EMPTY_PAGE = { baris: [], kursor: null };

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    cariLog: vi.fn().mockResolvedValue({ ...EMPTY_PAGE, di_luar_retensi_online: false }),
    cariLogNik: vi.fn().mockResolvedValue({ ...EMPTY_PAGE, di_luar_retensi_online: false }),
    bukaPayload: vi.fn(),
    // The export is a browser download, not JSON; the smoke test never triggers it.
    eksporLog: vi.fn(),
    cariAuditAkses: vi.fn().mockResolvedValue(EMPTY_PAGE),
    kuota: vi.fn().mockResolvedValue([]),
    upstreamPools: vi.fn().mockResolvedValue([]),
    listProviders: vi.fn().mockResolvedValue([]),
    listSecrets: vi
      .fn()
      .mockResolvedValue([{ nama: 'USER_ID', tersamar: true, dipakai_oleh_provider_count: 2 }]),
    ...overrides,
  } as ApiClient;
}

let en: i18n;

beforeAll(async () => {
  en = createI18n({ throwOnMissingKey: true });
  await loadLanguage(en, 'en');
});

describe('en smoke', () => {
  it('renders S11 log transaksi in en without a missing key', async () => {
    renderWithI18n(<LogPageScreen client={stubClient()} canOpenPayload />, { instance: en });

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Transaction logs');
  });

  it('renders S12 pencarian per NIK in en without a missing key', async () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />, { instance: en });

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent(
      'Search logs by NIK',
    );
  });

  it('renders S13 audit akses in en without a missing key', async () => {
    renderWithI18n(<AuditScreen client={stubClient()} />, { instance: en });

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Access audit');
  });

  it('renders the quota tracker in en without a missing key', async () => {
    renderWithI18n(<QuotaScreen client={stubClient()} />, { instance: en });

    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Quota');
  });

  it('renders S02 providers, the provider form and S05 secrets in en without a missing key', async () => {
    const client = stubClient();
    const list = renderWithI18n(<ProviderListScreen client={client} />, { instance: en });
    expect(await screen.findByText('No providers yet')).toBeInTheDocument();
    list.unmount();

    const form = renderWithI18n(<ProviderFormScreen client={client} />, { instance: en });
    expect(screen.getByLabelText('Body template (JSON)')).toBeInTheDocument();
    form.unmount();

    renderWithI18n(<SecretScreen client={client} />, { instance: en });
    expect(await screen.findByText('2 providers')).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Secrets');
  });
});
