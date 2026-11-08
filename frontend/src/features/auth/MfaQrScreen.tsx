import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { QRCodeSVG } from 'qrcode.react';
import { Card } from '../../components/m3/Card';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';

export interface MfaQrScreenProps {
  otpauthUri?: string;
  setupKey?: string;
  account?: string;
  issuer?: string;
  onConfirm?: (code: string) => void | Promise<void>;
  loading?: boolean;
  error?: string | null;
}

/** S17b — pindai QR + konfirmasi. Secret hanya state komponen. */
export function MfaQrScreen({
  otpauthUri = '',
  setupKey = '',
  account = '',
  issuer = 'PoolRouter',
  onConfirm,
  loading = false,
  error = null,
}: MfaQrScreenProps): ReactNode {
  const { t } = useTranslation();
  const snackbar = useSnackbar();
  const [code, setCode] = useState('');

  const copy = async (value: string, message: string) => {
    try {
      await navigator.clipboard.writeText(value);
      snackbar.notify(message);
    } catch {
      snackbar.notify(t('auth.mfa.qr.copy_failed'));
    }
  };

  const grouped = setupKey.replace(/\s+/g, '').match(/.{1,4}/g)?.join(' ') ?? setupKey;

  return (
    <section className="flex min-h-screen items-center justify-center bg-[var(--md-sys-color-surface)] p-4">
      <SnackbarRegion queue={snackbar.queue} />
      <Card className="w-full max-w-[428px] p-8">
        <p className="typescale-label-medium text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.qr.step')}
        </p>
        <h1 className="typescale-title-large">{t('auth.mfa.qr.title')}</h1>
        <div
          role="img"
          aria-label={t('auth.mfa.qr.qr_aria')}
          className="flex items-center justify-center rounded border border-dashed border-[var(--md-sys-color-outline)] p-4"
        >
          {otpauthUri ? (
            <QRCodeSVG
              value={otpauthUri}
              size={200}
              marginSize={4}
              fgColor="var(--md-sys-color-on-surface)"
              bgColor="var(--md-sys-color-surface-container-lowest)"
            />
          ) : null}
        </div>
        <p className="typescale-body-small text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.qr.manual_hint')}
        </p>
        <div className="rounded border border-[var(--md-sys-color-outline)] p-3 font-mono text-sm tracking-widest">
          {grouped}
        </div>
        <Button
          variant="filled-tonal"
          onClick={() => copy(setupKey.replace(/\s+/g, ''), t('auth.mfa.qr.copied_setup'))}
        >
          {t('common.action.copy')}
        </Button>
        <p className="typescale-body-small text-[var(--md-sys-color-on-surface-variant)]">
          {t('auth.mfa.qr.account', { account, issuer })}
        </p>
        <TextField
          label={t('auth.mfa.qr.code_label')}
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
          isDisabled={loading || code.replace(/\s+/g, '').length < 6}
          onClick={() => onConfirm?.(code.replace(/\s+/g, ''))}
        >
          {loading ? t('auth.mfa.code.verifying') : t('auth.mfa.qr.confirm')}
        </Button>
      </Card>
    </section>
  );
}

export default MfaQrScreen;
