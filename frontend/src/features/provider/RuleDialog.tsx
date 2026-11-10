import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { Select, SelectItem } from '../../components/m3/Select';
import { TextArea, TextField } from '../../components/m3/TextField';
import {
  buildParameter,
  draftFromParameter,
  emptyRuleDraft,
  RULE_JENIS,
  type ParameterRule,
  type RuleDraft,
  type RuleErrors,
  type RuleField,
} from './ruleEditor';

const JENIS_KEY = {
  regex: 'provider.rules.type.regex',
  enum: 'provider.rules.type.enum',
  panjang: 'provider.rules.type.length',
  format_tanggal: 'provider.rules.type.date_format',
} as const;

const ERROR_KEY = {
  wajib: 'provider.rules.editor.error.wajib',
  duplikat: 'provider.rules.editor.error.duplikat',
  bilangan: 'provider.rules.editor.error.bilangan',
  rentang: 'provider.rules.editor.error.rentang',
} as const;

const NONE = 'none';

export interface RuleDialogProps {
  /** Parameter yang diubah; `null` = parameter baru. Dialog dipasang hanya saat terbuka. */
  initial: ParameterRule | null;
  /** Indeks baris yang diubah, supaya namanya sendiri bukan duplikat. */
  editingIndex: number | null;
  existingNames: readonly string[];
  onSave: (row: ParameterRule) => void;
  onCancel: () => void;
}

/**
 * Editor satu parameter dan rule-nya (Penpot S04, `Ubah` dan `Tambah parameter`). Kolom yang tampil
 * mengikuti jenis rule; hanya jenis di kontrak (`RuleJenis`) yang bisa dipilih. Tanpa `<form>`:
 * dialog ini dirender di dalam form provider (lewat portal React) dan submit-nya akan merambat ke
 * form itu.
 */
export function RuleDialog({
  initial,
  editingIndex,
  existingNames,
  onSave,
  onCancel,
}: RuleDialogProps): ReactNode {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<RuleDraft>(() =>
    initial ? draftFromParameter(initial) : emptyRuleDraft(),
  );
  const [errors, setErrors] = useState<RuleErrors>({});

  const set = <K extends RuleField>(field: K, value: RuleDraft[K]) =>
    setDraft((prev) => ({ ...prev, [field]: value }));
  const err = (field: RuleField) => {
    const code = errors[field];
    return { isInvalid: code !== undefined, errorMessage: code ? t(ERROR_KEY[code]) : undefined };
  };

  function save() {
    const result = buildParameter(draft, existingNames, editingIndex);
    if (!result.ok) {
      setErrors(result.errors);
      return;
    }
    onSave(result.row);
  }

  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={initial ? t('provider.rules.editor.title_edit') : t('provider.rules.editor.title_add')}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onCancel}>
            {t('common.action.cancel')}
          </Button>
          <Button onPress={save}>{t('provider.rules.editor.save')}</Button>
        </>
      }
    >
      <div className="flex flex-col gap-4 pt-4">
        <TextField
          label={t('provider.rules.editor.nama')}
          description={t('provider.rules.editor.nama_help')}
          value={draft.nama}
          onChange={(v) => set('nama', v)}
          {...err('nama')}
        />
        <TextField
          label={t('provider.rules.editor.nilai')}
          description={t('provider.rules.editor.nilai_help')}
          value={draft.nilai}
          onChange={(v) => set('nilai', v)}
        />
        <Select
          label={t('provider.rules.editor.jenis')}
          selectedKey={draft.jenis === '' ? NONE : draft.jenis}
          onSelectionChange={(key) => set('jenis', key === NONE ? '' : (key as RuleDraft['jenis']))}
          className="w-full"
        >
          <SelectItem id={NONE}>{t('provider.rules.editor.jenis_none')}</SelectItem>
          {RULE_JENIS.map((jenis) => (
            <SelectItem key={jenis} id={jenis}>
              {t(JENIS_KEY[jenis])}
            </SelectItem>
          ))}
        </Select>
        {draft.jenis === 'regex' ? (
          <TextField
            label={t('provider.rules.editor.pola')}
            description={t('provider.rules.editor.pola_help')}
            value={draft.pola}
            onChange={(v) => set('pola', v)}
            {...err('pola')}
          />
        ) : null}
        {draft.jenis === 'enum' ? (
          <TextArea
            label={t('provider.rules.editor.enum')}
            description={t('provider.rules.editor.enum_help')}
            value={draft.nilaiDiizinkan}
            onChange={(v) => set('nilaiDiizinkan', v)}
            rows={4}
            {...err('nilaiDiizinkan')}
          />
        ) : null}
        {draft.jenis === 'panjang' ? (
          <div className="grid grid-cols-2 gap-4">
            <TextField
              label={t('provider.rules.editor.panjang_min')}
              inputMode="numeric"
              value={draft.panjangMin}
              onChange={(v) => set('panjangMin', v)}
              {...err('panjangMin')}
            />
            <TextField
              label={t('provider.rules.editor.panjang_maks')}
              inputMode="numeric"
              value={draft.panjangMaks}
              onChange={(v) => set('panjangMaks', v)}
              {...err('panjangMaks')}
            />
          </div>
        ) : null}
        {draft.jenis === 'format_tanggal' ? (
          <TextField
            label={t('provider.rules.editor.format')}
            description={t('provider.rules.editor.format_help')}
            value={draft.formatTanggal}
            onChange={(v) => set('formatTanggal', v)}
            {...err('formatTanggal')}
          />
        ) : null}
      </div>
    </Dialog>
  );
}
