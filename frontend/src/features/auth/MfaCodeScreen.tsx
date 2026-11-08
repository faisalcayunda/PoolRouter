import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Card } from '../../components/m3/Card';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

export interface MfaCodeScreenProps {
  username?: string;
  onSubmit?: (code: string) => void | Promise<void>;
  onBack?: () => void;
  loading?: boolean;
  error?: string | null;
}

type Mode = 'totp' | 'recovery';

/** S17 — kode authenticator (6 digit) atau, dalam mode pemulihan, kode 10 karakter. */
export function MfaCodeScreen({
  username,
  onSubmit,
  onBack,
  loading = false,
  error = null,
}: MfaCodeScreenProps): ReactNode {
  const { t } = useTranslation();
  const [mode, setMode] = useState<Mode>('totp');
  const [code, setCode] = useState('');
  // S17b menormalkan sebelum kirim: TOTP membuang spasi, kode pemulihan juga membesarkan huruf.
  const normalized =
    mode === 'totp'
      ? code.replace(/\s+/g, '')
      : code.toUpperCase().replace(/[\s-]/g, '');
  const valid = normalized.length === (mode === 'totp' ? 6 : 10);

  const switchMode = () => {
    setMode(mode === 'totp' ? 'recovery' : 'totp');
    setCode('');
  };

  return (
    <section className="flex min-h-screen items-center justify-center bg-[var(--md-sys-color-surface)] p-4">
      <Card className="w-full max-w-[428px] p-8">
        <h1 className="typescale-title-large">
          {t(mode === 'totp' ? 'auth.mfa.code.title' : 'auth.mfa.code.recovery_title')}
        </h1>
        <p className="typescale-body-medium text-[var(--md-sys-color-on-surface-variant)]">
          {mode === 'totp'
            ? t('auth.mfa.code.subtitle', { username: username ?? '' })
            : t('auth.mfa.code.recovery_subtitle')}
        </p>
        <TextField
          label={t(
            mode === 'totp' ? 'auth.mfa.code.field_label' : 'auth.mfa.code.recovery_field_label',
          )}
          inputMode={mode === 'totp' ? 'numeric' : 'text'}
          autoComplete={mode === 'totp' ? 'one-time-code' : 'off'}
          maxLength={mode === 'totp' ? 6 : 12}
          pattern={mode === 'totp' ? '[0-9]{6}' : undefined}
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
        <Button variant="text" onClick={switchMode}>
          {t(mode === 'totp' ? 'auth.mfa.code.use_recovery' : 'auth.mfa.code.use_totp')}
        </Button>
        <Button variant="text" onClick={onBack}>
          {t('auth.mfa.code.back_to_login')}
        </Button>
      </Card>
    </section>
  );
}

export default MfaCodeScreen;
