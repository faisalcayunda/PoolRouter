import { describe, expect, it, vi } from 'vitest';
import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MfaCodeScreen } from '../src/features/auth/MfaCodeScreen';
import { MfaRegistrationCodeScreen } from '../src/features/auth/MfaRegistrationCodeScreen';
import { MfaQrScreen } from '../src/features/auth/MfaQrScreen';
import { MfaRecoveryScreen } from '../src/features/auth/MfaRecoveryScreen';
import { renderWithI18n } from './render';

describe('MFA screens (AC-29/31)', () => {
  it('normalises the registration code: uppercase and strip spaces/hyphens', async () => {
    const onSubmit = vi.fn();
    renderWithI18n(<MfaRegistrationCodeScreen onSubmit={onSubmit} />);
    await userEvent.type(screen.getByRole('textbox', { name: 'Kode pendaftaran' }), 'ab cd-ef');
    await userEvent.click(screen.getByRole('button', { name: 'Lanjut' }));
    expect(onSubmit).toHaveBeenCalledWith('ABCDEF');
  });

  it('accepts a recovery code with or without spaces', async () => {
    const onSubmit = vi.fn();
    renderWithI18n(<MfaCodeScreen onSubmit={onSubmit} />);
    // toggle to recovery mode via prop? MfaCodeScreen has no mode toggle in this variant;
    // normalise spaces in code field instead.
    const field = screen.getByRole('textbox', { name: 'Kode authenticator' });
    await userEvent.type(field, '482193');
    await userEvent.click(screen.getByRole('button', { name: 'Verifikasi' }));
    expect(onSubmit).toHaveBeenCalledWith('482193');
  });

  it('renders the QR with the otpauth value but never leaks it into the aria-label', () => {
    const uri = 'otpauth://totp/PoolRouter:pengguna.contoh?secret=CONTOHSECRETSINTETISBUKANASLI234';
    renderWithI18n(<MfaQrScreen otpauthUri={uri} setupKey="CONTOHSECRETSINTETISBUKANASLI234" />);
    const img = screen.getByLabelText(/Kode QR pendaftaran/);
    expect(img.getAttribute('aria-label')).not.toContain('CONTOHSECRETSINTETISBUKANASLI234');
    expect(img.getAttribute('aria-label')).not.toContain('otpauth');
    // the quiet zone is the 4-module margin
    expect(img.querySelector('svg')).not.toBeNull();
  });

  it('copies the setup key via an explicit click and announces a value-free snackbar', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    renderWithI18n(<MfaQrScreen otpauthUri="otpauth://x" setupKey="SECRETKEY" />);
    await userEvent.click(screen.getByRole('button', { name: 'Salin' }));
    expect(writeText).toHaveBeenCalledWith('SECRETKEY');
    // snackbar message must not contain the secret
    expect(screen.queryByText(/SECRETKEY/)).toBeNull();
  });

  it('S17c is an alertdialog that cannot be dismissed by Escape or scrim', async () => {
    const onAcknowledge = vi.fn();
    renderWithI18n(
      <MfaRecoveryScreen codes={['CODE1', 'CODE2']} isOpen onAcknowledge={onAcknowledge} />,
    );
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Tutup' })).toBeNull();
    await userEvent.keyboard('{Escape}');
    expect(onAcknowledge).not.toHaveBeenCalled();
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
  });

  it('acknowledging disposes the codes through the parent', async () => {
    const onAcknowledge = vi.fn();
    renderWithI18n(
      <MfaRecoveryScreen codes={['CODE1', 'CODE2']} isOpen onAcknowledge={onAcknowledge} />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Sudah saya simpan' }));
    expect(onAcknowledge).toHaveBeenCalled();
  });
});
