import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Card } from '../../components/m3/Card';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

export interface MfaRegistrationCodeScreenProps {
  onSubmit?: (code: string) => void | Promise<void>;
  onBack?: () => void;
  loading?: boolean;
  error?: string | null;
}

/** S17a — kode pendaftaran. Normalisasi: uppercase, strip spasi/hyphen. */
export function MfaRegistrationCodeScreen({
  onSubmit,
  onBack,
  loading = false,
  error = null,
}: MfaRegistrationCodeScreenProps): ReactNode {
  const { t } = useTranslation();
  const [code, setCode] = useState('');
  const normalized = code.toUpperCase().replace(/[\s-]/g, '');

  return (
    <section className="flex min-h-screen items-center justify-center bg-[var(--md-sys-color-surface)] p-4">
      <Card variant="container" className="w-full max-w-[428px] p-6 min-[480px]:p-9">
        <p className="typescale-label-medium text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.registration.step')}
        </p>
        <h1 className="typescale-title-large">{t('auth.mfa.registration.title')}</h1>
        <p className="typescale-body-medium text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.registration.subtitle')}
        </p>
        <TextField
          label={t('auth.mfa.registration.field_label')}
          inputMode="text"
          autoComplete="off"
          spellCheck="false"
          value={code}
          onChange={(value) => setCode(value)}
        />
        {error ? <InlineAlert tone="error">{error}</InlineAlert> : null}
        <Button
          variant="filled"
          className="w-full"
          isDisabled={loading || normalized.length === 0}
          onClick={() => onSubmit?.(normalized)}
        >
          {loading ? t('auth.mfa.code.verifying') : t('auth.mfa.registration.submit')}
        </Button>
        <Button variant="text" onClick={onBack}>
          {t('auth.mfa.code.back_to_login')}
        </Button>
      </Card>
    </section>
  );
}

export default MfaRegistrationCodeScreen;
