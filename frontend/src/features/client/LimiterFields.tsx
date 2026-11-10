import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { TextField } from '../../components/m3/TextField';
import {
  BURST_DEFAULT,
  BURST_MAX,
  BURST_MIN,
  NAMA_MAKS,
  REFILL_DEFAULT,
  REFILL_MAX,
  REFILL_MIN,
  type ClientFormValues,
  type LimiterErrors,
} from './clientForm';

/** Kolom nama dan limiter yang dipakai bersama dialog tambah dan form ubah client. */
export function LimiterFields({
  values,
  onChange,
  errors,
  showErrors,
}: {
  values: ClientFormValues;
  onChange: (next: ClientFormValues) => void;
  errors: LimiterErrors;
  showErrors: boolean;
}): ReactNode {
  const { t, i18n } = useTranslation();
  const refillRange = new Intl.NumberFormat(i18n.language, { minimumFractionDigits: 2 });
  const namaError =
    errors.nama === 'required'
      ? t('clients.error.name_required')
      : errors.nama === 'too_long'
        ? t('clients.error.name_too_long', { maks: NAMA_MAKS })
        : null;
  return (
    <>
      <TextField
        label={t('client.form.name')}
        description={t('client.form.name_help')}
        value={values.nama}
        onChange={(nama) => onChange({ ...values, nama })}
        autoComplete="off"
        isInvalid={showErrors && namaError !== null}
        errorMessage={namaError}
      />
      <TextField
        label={t('clients.form.burst')}
        description={t('clients.form.burst_help', {
          min: BURST_MIN,
          maks: BURST_MAX,
          bawaan: BURST_DEFAULT,
        })}
        value={values.burst}
        onChange={(burst) => onChange({ ...values, burst })}
        inputMode="numeric"
        autoComplete="off"
        isInvalid={showErrors && errors.burst !== undefined}
        errorMessage={
          errors.burst ? t('clients.error.burst_range', { min: BURST_MIN, maks: BURST_MAX }) : null
        }
      />
      <TextField
        label={t('clients.form.refill')}
        description={t('clients.form.refill_help', {
          min: refillRange.format(REFILL_MIN),
          maks: refillRange.format(REFILL_MAX),
          bawaan: refillRange.format(REFILL_DEFAULT),
        })}
        value={values.refill}
        onChange={(refill) => onChange({ ...values, refill })}
        inputMode="decimal"
        autoComplete="off"
        isInvalid={showErrors && errors.refill !== undefined}
        errorMessage={
          errors.refill
            ? t('clients.error.refill_range', {
                min: refillRange.format(REFILL_MIN),
                maks: refillRange.format(REFILL_MAX),
              })
            : null
        }
      />
    </>
  );
}
