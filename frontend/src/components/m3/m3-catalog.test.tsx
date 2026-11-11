import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState, type ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { parseDate, parseTime, type DateValue } from '@internationalized/date';
import { UNSTABLE_ToastQueue } from 'react-aria-components';
import { AppI18nProvider } from '../../i18n/provider';
import { createI18n } from '../../i18n/i18n';
import { Breadcrumbs } from './Breadcrumbs';
import { Button } from './Button';
import { Card } from './Card';
import { CircularProgress } from './CircularProgress';
import { DatePicker, DateRangePicker } from './DatePicker';
import { Dialog } from './Dialog';
import { InlineAlert } from './InlineAlert';
import { LinearProgress } from './LinearProgress';
import { NavigationDrawer } from './NavigationDrawer';
import { Pagination } from './Pagination';
import { SideSheet } from './SideSheet';
import { Snackbar } from './Snackbar';
import { StatusLabel } from './StatusLabel';
import { TimeField } from './TimeField';
import { Tooltip, TooltipTrigger } from './Tooltip';
import { TopAppBar } from './TopAppBar';

const i18n = createI18n({ throwOnMissingKey: true });

/** Catalog text comes from the usage site; the provider makes React Aria format as id-ID. */
function wrap(ui: ReactNode) {
  return render(<AppI18nProvider i18n={i18n}>{ui}</AppI18nProvider>);
}

describe('Card', () => {
  it('renders all three catalog variants and keeps its children', () => {
    const { rerender } = render(<Card variant="elevated">Isi kartu</Card>);
    expect(screen.getByText('Isi kartu').className).toContain('shadow-level1');
    rerender(<Card variant="filled">Isi kartu</Card>);
    expect(screen.getByText('Isi kartu').className).toContain('bg-surface-container-highest');
    rerender(<Card variant="outlined">Isi kartu</Card>);
    expect(screen.getByText('Isi kartu').className).toContain('border-outline-variant');
  });
});

describe('Dialog', () => {
  // `Modal` takes its open state from the caller (DialogTrigger or isOpen+onOpenChange);
  // every dismissal path reports through that one callback.
  const renderDialog = (onOpenChange = vi.fn()) =>
    render(
      <Dialog isOpen onOpenChange={onOpenChange} title="Nonaktifkan provider" closeLabel="Tutup">
        Antrean provider ini akan dibatalkan.
      </Dialog>,
    );

  it('is a dialog named by its title with focus trapped inside', async () => {
    renderDialog();
    const dialog = screen.getByRole('dialog', { name: 'Nonaktifkan provider' });
    expect(dialog).toBeVisible();
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
  });

  it('closes on Escape', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    renderDialog(onOpenChange);
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('closes from the labelled close button', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    renderDialog(onOpenChange);
    await user.click(screen.getByRole('button', { name: 'Tutup' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe('SideSheet', () => {
  it('standard variant docks as a labelled region, not a dialog', () => {
    render(<SideSheet title="Detail transaksi">Isi</SideSheet>);
    expect(screen.getByRole('heading', { name: 'Detail transaksi' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('complementary', { name: 'Detail transaksi' })).toBeInTheDocument();
  });

  it('modal variant is a named dialog that closes on Escape', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <SideSheet
        isOpen
        onOpenChange={onOpenChange}
        title="Detail transaksi"
        variant="modal"
        closeLabel="Tutup"
      >
        Isi
      </SideSheet>,
    );
    expect(screen.getByRole('dialog', { name: 'Detail transaksi' })).toBeInTheDocument();
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe('Snackbar', () => {
  /** The queue lives in component state: RAC requires a single instance per region. */
  function Harness() {
    const [queue] = useState(
      () => new UNSTABLE_ToastQueue<{ message: string }>({ maxVisibleToasts: 3 }),
    );
    return (
      <>
        <Button onPress={() => queue.add({ message: 'Secret disalin' })}>Simpan</Button>
        <Snackbar
          queue={queue}
          closeLabel="Tutup"
          renderContent={(content) => <span>{content.message}</span>}
        />
      </>
    );
  }

  it('announces the message in a toast region and closes with the labelled button', async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole('button', { name: 'Simpan' }));

    expect(await screen.findByText('Secret disalin')).toBeInTheDocument();
    expect(screen.getByRole('region')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Tutup' }));
    await waitFor(() => expect(screen.queryByText('Secret disalin')).not.toBeInTheDocument());
  });
});

describe('Tooltip', () => {
  it('appears on keyboard focus and keeps its text equal to the accessible label (§7.5)', async () => {
    const user = userEvent.setup();
    wrap(
      <TooltipTrigger>
        <Button aria-label="Salin secret">x</Button>
        <Tooltip>Salin secret</Tooltip>
      </TooltipTrigger>,
    );
    const trigger = screen.getByRole('button', { name: 'Salin secret' });
    await user.tab();
    expect(trigger).toHaveFocus();
    // The label is present with or without the tooltip, so the tooltip is never the only name.
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Salin secret');
  });
});

describe('DatePicker (DV-7, id-ID)', () => {
  it('has a visible label and a labelled segment group', () => {
    wrap(
      <DatePicker
        label="Tanggal mulai"
        calendarLabel="Kalender"
        defaultValue={parseDate('2026-11-03')}
      />,
    );
    expect(screen.getByText('Tanggal mulai')).toBeVisible();
    expect(screen.getByRole('group', { name: 'Tanggal mulai' })).toBeInTheDocument();
  });

  it('orders and labels the segments for id-ID (dd/mm/yyyy)', () => {
    wrap(
      <DatePicker
        label="Tanggal"
        calendarLabel="Kalender"
        defaultValue={parseDate('2026-11-03')}
      />,
    );
    const group = screen.getByRole('group', { name: 'Tanggal' });
    const segments = within(group).getAllByRole('spinbutton');
    // Segment labels come from the id-ID CLDR strings; the separator trails the name.
    expect(segments.map((s) => s.getAttribute('aria-label')?.replace(/[,\s]+$/, ''))).toEqual([
      'hari',
      'bulan',
      'tahun',
    ]);
    expect(within(group).getAllByText('/')).toHaveLength(2);
    expect(segments[0]).toHaveTextContent('3');
  });

  it('accepts typed input segment by segment and reports an id-ID date', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    wrap(<DatePicker label="Tanggal" calendarLabel="Kalender" onChange={onChange} />);
    const segments = within(screen.getByRole('group', { name: 'Tanggal' })).getAllByRole(
      'spinbutton',
    );
    // Typing a full segment advances to the next one, so no explicit ArrowRight.
    await user.click(segments[0]!);
    await user.keyboard('05');
    await user.keyboard('11');
    await user.keyboard('2026');
    await waitFor(() => {
      const value = onChange.mock.calls.at(-1)?.[0] as DateValue | null;
      expect(value?.toString()).toBe('2026-11-05');
    });
  });

  it('opens the calendar from the trigger by keyboard', async () => {
    const user = userEvent.setup();
    wrap(
      <DatePicker
        label="Tanggal"
        calendarLabel="Kalender"
        defaultValue={parseDate('2026-11-03')}
      />,
    );
    // RAC labels the trigger with both the calendar label and the field label.
    const trigger = screen.getByRole('button', { name: /Kalender/ });
    trigger.focus();
    await user.keyboard('{Enter}');
    expect(await screen.findByRole('grid')).toBeInTheDocument();
  });
});

describe('DateRangePicker (DV-7)', () => {
  it('has a visible label and a labelled range group', () => {
    wrap(<DateRangePicker label="Rentang waktu" calendarLabel="Kalender" separator="s/d" />);
    expect(screen.getByText('Rentang waktu')).toBeVisible();
    expect(screen.getAllByRole('group')[0]).toHaveAccessibleName(/Rentang waktu/);
  });

  it('wraps only below the expanded breakpoint (ARC-244)', () => {
    wrap(<DateRangePicker label="Rentang waktu" calendarLabel="Kalender" separator="s/d" />);
    const cls = screen.getAllByRole('group')[0]!.className;
    expect(cls).toContain('flex-wrap');
    expect(cls).toContain('expanded:h-14');
    expect(cls).toContain('expanded:flex-nowrap');
  });

  it('opens the range calendar by keyboard', async () => {
    const user = userEvent.setup();
    wrap(<DateRangePicker label="Rentang waktu" calendarLabel="Kalender" separator="s/d" />);
    const trigger = screen.getByRole('button', { name: /Kalender/ });
    trigger.focus();
    await user.keyboard('{Enter}');
    expect(await screen.findByRole('grid')).toBeInTheDocument();
  });
});

describe('TimeField', () => {
  it('has a visible label and WIB hour/minute segments', async () => {
    const user = userEvent.setup();
    wrap(<TimeField label="Jam mulai" defaultValue={parseTime('14:30')} />);
    expect(screen.getByText('Jam mulai')).toBeVisible();
    // The field nests a group per input, so take the first one and read its segments.
    const group = screen.getAllByRole('group', { name: 'Jam mulai' })[0]!;
    const segments = within(group).getAllByRole('spinbutton');
    expect(segments.map((s) => s.getAttribute('aria-label')?.split(',')[0]?.trim())).toEqual([
      'jam',
      'menit',
    ]);
    expect(segments[0]).toHaveAttribute('aria-valuetext', '14');
    await user.click(segments[0]!);
    expect(group.contains(document.activeElement)).toBe(true);
  });
});

describe('Pagination', () => {
  const labels = { prevLabel: 'Halaman sebelumnya', nextLabel: 'Halaman berikutnya' };

  it('disables prev on the first page and steps forward', async () => {
    const user = userEvent.setup();
    const onPageChange = vi.fn();
    render(<Pagination page={1} pageCount={3} onPageChange={onPageChange} {...labels} />);
    expect(screen.getByRole('button', { name: labels.prevLabel })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: labels.nextLabel }));
    expect(onPageChange).toHaveBeenCalledWith(2);
  });

  it('disables next on the last page and steps back', async () => {
    const user = userEvent.setup();
    const onPageChange = vi.fn();
    render(<Pagination page={3} pageCount={3} onPageChange={onPageChange} {...labels} />);
    expect(screen.getByRole('button', { name: labels.nextLabel })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: labels.prevLabel }));
    expect(onPageChange).toHaveBeenCalledWith(2);
  });
});

describe('LinearProgress', () => {
  it('exposes the value when determinate', () => {
    render(<LinearProgress aria-label="Progres provisioning" value={40} />);
    expect(screen.getByRole('progressbar', { name: 'Progres provisioning' })).toHaveAttribute(
      'aria-valuenow',
      '40',
    );
  });

  it('is indeterminate when no value is given', () => {
    render(<LinearProgress aria-label="Memuat" />);
    const bar = screen.getByRole('progressbar', { name: 'Memuat' });
    expect(bar.querySelector('[data-indeterminate]')).not.toBeNull();
  });
});

describe('CircularProgress', () => {
  it('is a labelled progressbar carrying the determinate value', () => {
    render(<CircularProgress aria-label="Menyimpan bahasa" value={60} />);
    expect(screen.getByRole('progressbar', { name: 'Menyimpan bahasa' })).toHaveAttribute(
      'aria-valuenow',
      '60',
    );
  });

  it('only spins while indeterminate', () => {
    const { rerender } = render(<CircularProgress aria-label="Memuat" />);
    const svgClass = () =>
      screen.getByRole('progressbar').querySelector('svg')?.getAttribute('class');
    expect(svgClass()).toContain('m3-spin');
    rerender(<CircularProgress aria-label="Memuat" value={10} />);
    expect(svgClass()).not.toContain('m3-spin');
  });
});

describe('NavigationDrawer', () => {
  it('standard variant is a labelled complementary landmark with no dialog', () => {
    render(
      <NavigationDrawer variant="standard" aria-label="Navigasi utama">
        <a href="/">Beranda</a>
      </NavigationDrawer>,
    );
    expect(screen.getByRole('complementary', { name: 'Navigasi utama' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('modal variant renders nothing closed, then a labelled dialog that traps focus', async () => {
    const onOpenChange = vi.fn();
    const { rerender } = render(
      <>
        <button type="button">Di luar drawer</button>
        <NavigationDrawer variant="modal" isOpen={false} aria-label="Navigasi utama">
          <a href="/">Beranda</a>
        </NavigationDrawer>
      </>,
    );
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Di luar drawer' })).toBeInTheDocument();

    rerender(
      <>
        <button type="button">Di luar drawer</button>
        <NavigationDrawer
          variant="modal"
          isOpen
          aria-label="Navigasi utama"
          onOpenChange={onOpenChange}
        >
          <a href="/">Beranda</a>
        </NavigationDrawer>
      </>,
    );
    const dialog = screen.getByRole('dialog', { name: 'Navigasi utama' });
    // Modal semantics: focus lands inside and the background leaves the a11y tree.
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    expect(screen.queryByRole('button', { name: 'Di luar drawer' })).not.toBeInTheDocument();
  });

  it('modal variant closes on Escape through onOpenChange', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <NavigationDrawer
        variant="modal"
        isOpen
        aria-label="Navigasi utama"
        onOpenChange={onOpenChange}
      >
        <a href="/">Beranda</a>
      </NavigationDrawer>,
    );
    await user.keyboard('{Escape}');
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe('TopAppBar', () => {
  it('renders the app title as text, leaving the level-1 heading to the screen', () => {
    // Handoff spec section 13.1: the TopAppBar title is not a heading, so `<main>` can open with
    // the screen's own `h1` without the document ending up with two.
    render(<TopAppBar title="Log transaksi" leading={<Button aria-label="Buka menu">m</Button>} />);
    expect(screen.getByRole('banner')).toBeInTheDocument();
    expect(screen.queryAllByRole('heading')).toHaveLength(0);
    expect(screen.getByText('Log transaksi')).toBeVisible();
  });
});

describe('StatusLabel', () => {
  it('always shows text so colour is not the only signal (§7.5)', () => {
    render(<StatusLabel tone="error">Gagal</StatusLabel>);
    expect(screen.getByText('Gagal')).toBeVisible();
  });

  it('carries a tone icon by default, which can be hidden explicitly', () => {
    const { rerender } = render(<StatusLabel tone="success">Selesai</StatusLabel>);
    const glyph = () => screen.getByText('Selesai').querySelector('svg');
    expect(glyph()).not.toBeNull();
    rerender(
      <StatusLabel tone="success" icon={null}>
        Selesai
      </StatusLabel>,
    );
    expect(glyph()).toBeNull();
  });
});

describe('InlineAlert', () => {
  it('is an alert with heading, message and a retry action', async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    render(
      <InlineAlert
        tone="error"
        heading="Gagal memuat"
        action={<Button onPress={onRetry}>Coba lagi</Button>}
      >
        Periksa koneksi Anda.
      </InlineAlert>,
    );
    const alert = screen.getByRole('alert');
    expect(alert).toHaveAttribute('aria-live', 'polite');
    expect(within(alert).getByText('Gagal memuat')).toBeInTheDocument();
    expect(within(alert).getByText('Periksa koneksi Anda.')).toBeInTheDocument();
    await user.click(within(alert).getByRole('button', { name: 'Coba lagi' }));
    expect(onRetry).toHaveBeenCalled();
  });

  it('announces a save failure assertively', () => {
    render(
      <InlineAlert tone="error" live="assertive">
        Bahasa gagal disimpan.
      </InlineAlert>,
    );
    expect(screen.getByRole('alert')).toHaveAttribute('aria-live', 'assertive');
  });
});

describe('Breadcrumbs', () => {
  it('is a labelled trail whose last item is the current page, not a link', () => {
    render(
      <Breadcrumbs
        aria-label="Jejak navigasi"
        items={[{ label: 'Log transaksi', href: '/log' }, { label: 'Detail' }]}
      />,
    );
    const nav = screen.getByRole('navigation', { name: 'Jejak navigasi' });
    expect(within(nav).getByRole('link', { name: 'Log transaksi' })).toBeInTheDocument();
    expect(within(nav).getByText('Detail')).toHaveAttribute('aria-current', 'page');
    expect(within(nav).queryByRole('link', { name: 'Detail' })).not.toBeInTheDocument();
  });
});
