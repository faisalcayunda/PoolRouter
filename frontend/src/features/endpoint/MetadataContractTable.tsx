import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { ScrollArea } from '../common/ScrollArea';
import { tDynamic } from './MetadataFieldsEditor';
import type { MetadataRowDraft } from './endpointForm';

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

/**
 * Ringkasan kontrak metadata (S07b): satu baris per field dengan nama, tipe, wajib atau opsional,
 * dan rule. Dibaca dari draft form, jadi ikut berubah saat editor di bawahnya diubah. Rule enum
 * menampilkan nilai yang diizinkan; tipe lain menampilkan aturan validasinya atau strip.
 */
export function MetadataContractTable({ rows }: { rows: MetadataRowDraft[] }): ReactNode {
  const { t, i18n } = useTranslation();
  const named = rows.filter((row) => row.nama_field.trim() !== '');
  if (named.length === 0) return null;

  return (
    <ScrollArea label={t('endpoint.form.section.metadata')}>
      <div className="table-card">
        <table className="w-full border-collapse text-left">
          <caption className="sr-only">{t('endpoint.form.section.metadata')}</caption>
          <thead>
            <tr className="typescale-title-small text-on-surface-variant">
              <th scope="col" className={TH}>
                {t('metadata.col.name')}
              </th>
              <th scope="col" className={TH}>
                {t('metadata.col.type')}
              </th>
              <th scope="col" className={TH}>
                {t('metadata.col.required')}
              </th>
              <th scope="col" className={TH}>
                {t('metadata.col.rule')}
              </th>
            </tr>
          </thead>
          <tbody>
            {named.map((row) => {
              const rule = row.tipe === 'enum' ? row.nilai_diizinkan : row.aturan_validasi;
              return (
                <tr key={row.key} className="border-b border-outline-variant text-on-surface">
                  <td data-label={t('metadata.col.name')} className={`${TD} font-code`}>
                    {row.nama_field}
                  </td>
                  <td data-label={t('metadata.col.type')} className={TD}>
                    {tDynamic(i18n, `endpoint.metadata.tipe_option.${row.tipe}`)}
                  </td>
                  <td data-label={t('metadata.col.required')} className={TD}>
                    <StatusLabel tone="neutral">
                      {row.wajib ? t('common.state.required') : t('common.state.optional')}
                    </StatusLabel>
                  </td>
                  <td data-label={t('metadata.col.rule')} className={TD}>
                    {rule.trim() === '' ? '–' : rule}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </ScrollArea>
  );
}
