import type { StatusLabelTone } from '../../components/m3/StatusLabel';

/** Access status → tone. The label always carries text; the tone only reinforces it (section 8). */
export function accessTone(status: string): StatusLabelTone {
  if (status === 'ok') return 'success';
  if (status === 'timeout' || status === 'gagal' || status === 'ditolak') return 'error';
  return 'neutral';
}
