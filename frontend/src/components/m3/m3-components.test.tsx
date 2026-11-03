import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { Button } from './Button';
import { Checkbox, CheckboxGroup } from './Checkbox';
import { Chip } from './Chip';
import { IconButton } from './IconButton';
import { Radio, RadioGroup } from './Radio';
import { Switch } from './Switch';
import { TextField } from './TextField';
import { Skeleton } from './Skeleton';
import { Divider } from './Divider';

describe('Button (M3 F0 variants)', () => {
  it('renders a button with accessible name and fires onPress', async () => {
    const onPress = vi.fn();
    render(<Button onPress={onPress}>Simpan</Button>);
    const btn = screen.getByRole('button', { name: 'Simpan' });
    await userEvent.click(btn);
    expect(onPress).toHaveBeenCalledOnce();
  });

  it('disabled button does not fire onPress', async () => {
    const onPress = vi.fn();
    render(
      <Button onPress={onPress} isDisabled>
        Simpan
      </Button>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Simpan' }));
    expect(onPress).not.toHaveBeenCalled();
  });
});

describe('IconButton (Addendum A 5.4)', () => {
  it('requires an accessible name (aria-label is mandatory)', () => {
    render(<IconButton aria-label="Tambah perangkat">+</IconButton>);
    expect(screen.getByRole('button', { name: 'Tambah perangkat' })).toBeInTheDocument();
  });

  // §7.5: icon-only controls carry a plain tooltip with the same words as the
  // accessible name. Both come from one prop, so they cannot drift.
  it('shows a tooltip with the same text as its accessible name', async () => {
    render(<IconButton aria-label="Tambah perangkat">+</IconButton>);
    const button = screen.getByRole('button', { name: 'Tambah perangkat' });
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
    await userEvent.tab();
    expect(button).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Tambah perangkat');
  });
});

describe('Checkbox / CheckboxGroup', () => {
  it('toggles via click and reflects state in the group', async () => {
    render(
      <CheckboxGroup label="Protokol">
        <Checkbox value="modbus">Modbus</Checkbox>
        <Checkbox value="mqtt">MQTT</Checkbox>
      </CheckboxGroup>,
    );
    const group = screen.getByRole('group', { name: 'Protokol' });
    expect(group).toBeInTheDocument();
    const modbus = screen.getByRole('checkbox', { name: 'Modbus' });
    expect(modbus).not.toBeChecked();
    await userEvent.click(modbus);
    expect(modbus).toBeChecked();
  });

  it('toggles via Space key (§3.7)', async () => {
    render(<Checkbox>Auto reconnect</Checkbox>);
    const cb = screen.getByRole('checkbox');
    cb.focus();
    await userEvent.keyboard(' ');
    expect(cb).toBeChecked();
  });
});

describe('Radio / RadioGroup', () => {
  it('selects one option and moves with arrows (§3.7)', async () => {
    render(
      <RadioGroup label="Baud rate" defaultValue="9600">
        <Radio value="9600">9600</Radio>
        <Radio value="115200">115200</Radio>
      </RadioGroup>,
    );
    expect(screen.getByRole('radio', { name: '9600' })).toBeChecked();
    expect(screen.getByRole('radio', { name: '115200' })).not.toBeChecked();
    screen.getByRole('radio', { name: '9600' }).focus();
    await userEvent.keyboard('{ArrowDown}');
    expect(screen.getByRole('radio', { name: '115200' })).toBeChecked();
  });
});

describe('Switch', () => {
  it('toggles via Space', async () => {
    render(<Switch>Enable logging</Switch>);
    const sw = screen.getByRole('switch');
    expect(sw).not.toBeChecked();
    sw.focus();
    await userEvent.keyboard(' ');
    expect(sw).toBeChecked();
  });
});

describe('TextField', () => {
  it('associates label and input; typing is reflected', async () => {
    render(<TextField label="Nama perangkat" />);
    const input = screen.getByLabelText('Nama perangkat');
    await userEvent.type(input, 'pump-01');
    expect(input).toHaveValue('pump-01');
  });
});

describe('Chip', () => {
  it('filter chip toggles with aria-pressed', async () => {
    const onClick = vi.fn();
    const { rerender } = render(
      <Chip variant="filter" selected={false} onClick={onClick}>
        Online
      </Chip>,
    );
    const chip = screen.getByRole('button', { name: 'Online' });
    expect(chip).toHaveAttribute('aria-pressed', 'false');
    await userEvent.click(chip);
    expect(onClick).toHaveBeenCalledOnce();
    rerender(
      <Chip variant="filter" selected onClick={onClick}>
        Online
      </Chip>,
    );
    expect(screen.getByRole('button', { name: 'Online' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('input chip exposes a labelled remove button', async () => {
    const onRemove = vi.fn();
    render(
      <Chip variant="input" onRemove={onRemove} removeLabel="Hapus filter status">
        status: online
      </Chip>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Hapus filter status' }));
    expect(onRemove).toHaveBeenCalledOnce();
  });
});

describe('Skeleton / Divider', () => {
  it('skeleton is hidden from assistive tech', () => {
    const { container } = render(<Skeleton className="h-4 w-40" />);
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
  });

  it('divider has separator role', () => {
    render(<Divider />);
    expect(screen.getByRole('separator')).toBeInTheDocument();
  });
});
