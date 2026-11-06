import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Card } from '../../components/m3/Card';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

export interface MfaCodeScreenProps {
  username?: string;
  onSubmit?: (code: string) => void | Promise<void>;
  onRecovery?: () => void;
  onBack?: () => void;
  loading?: boolean;
  error?: string | null;
}

/** S17 — kode authenticator (6 digit). */
export function MfaCodeScreen({
  username,
  onSubmit,
  onRecovery,
  onBack,
  loading = false,
  error = null,
}: MfaCodeScreenProps): ReactNode {
  const { t } = useTranslation();
  const [code, setCode] = useState('');
  const normalized = code.replace(/\s+/g, '');
  const valid = normalized.length === 6;

  return (
    <section className="flex min-h-screen items-center justify-center bg-[var(--md-sys-color-surface)] p-4">
      <Card className="w-full max-w-[428px] p-8">
        <h1 className="typescale-title-large">{t('auth.mfa.code.title')}</h1>
        <p className="typescale-body-medium text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.code.subtitle', { username: username ?? '' })}
        </p>
        <TextField
          label={t('auth.mfa.code.field_label')}
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={6}
          pattern="[0-9]{6}"
          spellCheck="false"
          value={code}
          onChange={(value) => setCode(value)}
        />
        {error ? <InlineAlert tone="error">{error}</InlineAlert> : null}
        <Button
          variant="filled"
          className="w-full"
          isDisabled={loading || !valid}
          onClick={() => onSubmit?.(normalized)}
        >
          {loading ? t('auth.mfa.code.verifying') : t('auth.mfa.code.submit')}
        </Button>
        <Button variant="text" onClick={onRecovery}>
          {t('auth.mfa.code.use_recovery')}
        </Button>
        <Button variant="text" onClick={onBack}>
          {t('auth.mfa.code.back_to_login')}
        </Button>
      </Card>
    </section>
  );
}

export default MfaCodeScreen;
