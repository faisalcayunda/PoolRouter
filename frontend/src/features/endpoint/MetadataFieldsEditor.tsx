import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Checkbox } from '../../components/m3/Checkbox';
import { Select, SelectItem } from '../../components/m3/Select';
import { TextField } from '../../components/m3/TextField';
import {
  emptyRow,
  KLASIFIKASI,
  NAMA_FIELD_MAKS,
  TIPE_METADATA,
  type DraftErrors,
  type FieldIssue,
  type MetadataRowDraft,
  type RowField,
} from './endpointForm';

/** `t` dengan kunci dinamis; kunci sudah dicek dengan `exists` sebelum dipanggil. */
export function tDynamic(instance: I18n, key: string, vars: Record<string, unknown> = {}): string {
  return (instance.t as (k: string, o?: Record<string, unknown>) => string)(key, vars);
}

/** Kalimat galat dari katalog; kode yang belum dikenal memakai kalimat umum dengan kodenya. */
export function issueMessage(instance: I18n, issue: FieldIssue): string {
  const vars = { kode: issue.kode, nama: issue.nama ?? '', batas: issue.batas ?? '' };
  const key = `endpoint.form.error.${issue.kode}`;
  return instance.exists(key)
    ? tDynamic(instance, key, vars)
    : tDynamic(instance, 'endpoint.form.error.umum', vars);
}

export interface MetadataFieldsEditorProps {
  rows: MetadataRowDraft[];
  errors: DraftErrors;
  onChange: (rows: MetadataRowDraft[]) => void;
  /** Dipanggil saat satu isian berubah, supaya galatnya hilang. */
  onTouch: (key: number, field: RowField) => void;
}

/**
 * Editor kontrak metadata (S07b). Setiap field adalah satu grup bernama "Field ke-n" supaya label
 * yang sama di baris berbeda tetap bisa dibedakan oleh pembaca layar. Field `Rahasia` tidak pernah
 * dikirim ke callback data balikan, jadi kotak itu dikunci mati dan dijelaskan, bukan disembunyikan.
 */
export function MetadataFieldsEditor({
  rows,
  errors,
  onChange,
  onTouch,
}: MetadataFieldsEditorProps): ReactNode {
  const { t, i18n } = useTranslation();

  function patch(key: number, change: Partial<MetadataRowDraft>, field: RowField) {
    onChange(rows.map((row) => (row.key === key ? { ...row, ...change } : row)));
    onTouch(key, field);
  }

  return (
    <div className="flex flex-col gap-4">
      {rows.length === 0 ? (
        <p className="typescale-body-medium text-on-surface-variant">
          {t('endpoint.metadata.empty')}
        </p>
      ) : null}
      {rows.map((row, index) => {
        const rowErrors = errors.rows[row.key] ?? {};
        const err = (field: RowField) => {
          const issue = rowErrors[field];
          return {
            isInvalid: issue !== undefined,
            errorMessage: issue ? issueMessage(i18n, issue) : undefined,
          };
        };
        const rahasia = row.klasifikasi === 'Rahasia';
        return (
          <div
            key={row.key}
            role="group"
            aria-label={t('endpoint.metadata.group', { nomor: index + 1 })}
          >
            <Card variant="outlined" className="flex flex-col gap-4">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 className="typescale-title-small">
                  {t('endpoint.metadata.group', { nomor: index + 1 })}
                </h3>
                <Button
                  variant="text"
                  destructive
                  onPress={() => onChange(rows.filter((r) => r.key !== row.key))}
                >
                  {t('endpoint.metadata.remove')}
                </Button>
              </div>
              <div data-row={row.key} data-row-field="nama_field">
                <TextField
                  label={t('endpoint.metadata.nama_field')}
                  description={t('endpoint.metadata.nama_field_help', { batas: NAMA_FIELD_MAKS })}
                  value={row.nama_field}
                  onChange={(v) => patch(row.key, { nama_field: v }, 'nama_field')}
                  autoComplete="off"
                  className="[&_input]:font-code"
                  {...err('nama_field')}
                />
              </div>
              <div className="flex flex-wrap gap-4">
                <Select
                  label={t('endpoint.metadata.tipe')}
                  selectedKey={row.tipe}
                  onSelectionChange={(key) => {
                    const found = TIPE_METADATA.find((v) => v === key);
                    if (found) patch(row.key, { tipe: found }, 'tipe');
                  }}
                >
                  {TIPE_METADATA.map((v) => (
                    <SelectItem key={v} id={v}>
                      {tDynamic(i18n, `endpoint.metadata.tipe_option.${v}`)}
                    </SelectItem>
                  ))}
                </Select>
                <Select
                  label={t('endpoint.metadata.klasifikasi')}
                  selectedKey={row.klasifikasi}
                  onSelectionChange={(key) => {
                    const found = KLASIFIKASI.find((v) => v === key);
                    if (found) patch(row.key, { klasifikasi: found }, 'klasifikasi');
                  }}
                >
                  {KLASIFIKASI.map((v) => (
                    <SelectItem key={v} id={v}>
                      {tDynamic(i18n, `endpoint.metadata.klasifikasi_option.${v}`)}
                    </SelectItem>
                  ))}
                </Select>
              </div>
              {row.tipe === 'enum' ? (
                <div data-row={row.key} data-row-field="nilai_diizinkan">
                  <TextField
                    label={t('endpoint.metadata.nilai_diizinkan')}
                    description={t('endpoint.metadata.nilai_diizinkan_help')}
                    value={row.nilai_diizinkan}
                    onChange={(v) => patch(row.key, { nilai_diizinkan: v }, 'nilai_diizinkan')}
                    autoComplete="off"
                    {...err('nilai_diizinkan')}
                  />
                </div>
              ) : null}
              <div data-row={row.key} data-row-field="deskripsi">
                <TextField
                  label={t('endpoint.metadata.deskripsi')}
                  value={row.deskripsi}
                  onChange={(v) => patch(row.key, { deskripsi: v }, 'deskripsi')}
                  autoComplete="off"
                  {...err('deskripsi')}
                />
              </div>
              <TextField
                label={t('endpoint.metadata.aturan_validasi')}
                description={t('endpoint.metadata.aturan_validasi_help')}
                value={row.aturan_validasi}
                onChange={(v) => patch(row.key, { aturan_validasi: v }, 'aturan_validasi')}
                autoComplete="off"
              />
              <div className="flex flex-col gap-1">
                <Checkbox
                  isSelected={row.wajib}
                  onChange={(v) => patch(row.key, { wajib: v }, 'wajib')}
                >
                  {t('endpoint.metadata.wajib')}
                </Checkbox>
                <Checkbox
                  isSelected={rahasia ? false : row.kirim_ke_data_balikan}
                  isDisabled={rahasia}
                  onChange={(v) =>
                    patch(row.key, { kirim_ke_data_balikan: v }, 'kirim_ke_data_balikan')
                  }
                >
                  {t('endpoint.metadata.kirim')}
                </Checkbox>
                {rahasia ? (
                  <p className="typescale-body-small text-on-surface-variant">
                    {t('endpoint.metadata.rahasia_note')}
                  </p>
                ) : null}
              </div>
            </Card>
          </div>
        );
      })}
      <div>
        <Button variant="filled-tonal" onPress={() => onChange([...rows, emptyRow()])}>
          {t('endpoint.metadata.add')}
        </Button>
      </div>
    </div>
  );
}
