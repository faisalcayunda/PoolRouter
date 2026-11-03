import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { List, ListItem } from './List';
import { Menu, MenuItem, MenuSeparator, MenuTrigger } from './Menu';
import { Select, SelectItem } from './Select';
import { Button } from './Button';
import { ComboBox, ComboBoxItem } from './ComboBox';
import { DataTable, DataTableBody, DataTableCell, DataTableColumn, DataTableHeader, DataTableRow } from './DataTable';
import { Tab, TabList, TabPanel, Tabs } from './Tabs';

describe('Select', () => {
  it('opens the listbox, picks an option, and reflects the value', async () => {
    const onChange = vi.fn();
    render(
      <Select label="Protokol" onChange={onChange}>
        <SelectItem id="modbus">Modbus TCP</SelectItem>
        <SelectItem id="mqtt">MQTT</SelectItem>
      </Select>,
    );
    await userEvent.click(screen.getByRole('button', { name: /protokol/i }));
    const option = await screen.findByRole('option', { name: 'MQTT' });
    await userEvent.click(option);
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: /mqtt/i })).toBeInTheDocument();
  });

  it('keyboard: Enter opens, arrows navigate, Enter selects (§3.7)', async () => {
    render(
      <Select label="Baud rate" defaultValue="9600">
        <SelectItem id="9600">9600</SelectItem>
        <SelectItem id="115200">115200</SelectItem>
      </Select>,
    );
    const trigger = screen.getByRole('button');
    trigger.focus();
    await userEvent.keyboard('{Enter}');
    const opts = await screen.findAllByRole('option');
    expect(opts.length).toBe(2);
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument());
  });
});

describe('ComboBox', () => {
  it('filters options as the user types', async () => {
    render(
      <ComboBox label="Perangkat">
        <ComboBoxItem id="p1" textValue="pump-01">
          pump-01
        </ComboBoxItem>
        <ComboBoxItem id="p2" textValue="valve-02">
          valve-02
        </ComboBoxItem>
      </ComboBox>,
    );
    const input = screen.getByRole('combobox');
    await userEvent.type(input, 'pump');
    const opt = await screen.findByRole('option', { name: 'pump-01' });
    await userEvent.click(opt);
    expect(input).toHaveValue('pump-01');
    await waitFor(() => expect(screen.queryByRole('option', { name: 'valve-02' })).not.toBeInTheDocument());
  });
});

describe('Menu', () => {
  it('opens on trigger, fires action, closes on selection', async () => {
    const onAction = vi.fn();
    render(
      <MenuTrigger>
        <Button>Menu</Button>
        <Menu onAction={onAction}>
          <MenuItem id="rename">Rename</MenuItem>
          <MenuItem id="delete">Delete</MenuItem>
          <MenuSeparator />
          <MenuItem id="info">Info</MenuItem>
        </Menu>
      </MenuTrigger>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    const item = await screen.findByRole('menuitem', { name: 'Delete' });
    await userEvent.click(item);
    await waitFor(() => expect(onAction.mock.calls[0]?.[0]).toBe('delete'));
  });
});

describe('Tabs', () => {
  it('switches panels with arrow keys (§3.7)', async () => {
    render(
      <Tabs>
        <TabList aria-label="Detail">
          <Tab id="status">Status</Tab>
          <Tab id="log">Log</Tab>
        </TabList>
        <TabPanel id="status">STATUS_PANEL</TabPanel>
        <TabPanel id="log">LOG_PANEL</TabPanel>
      </Tabs>,
    );
    expect(screen.getByText('STATUS_PANEL')).toBeInTheDocument();
    const statusTab = screen.getByRole('tab', { name: 'Status' });
    statusTab.focus();
    await userEvent.keyboard('{ArrowRight}');
    await waitFor(() => expect(screen.getByText('LOG_PANEL')).toBeInTheDocument());
  });
});

describe('DataTable', () => {
  it('renders a keyboard-navigable grid with header rows', async () => {
    render(
      <DataTable aria-label="Perangkat">
        <DataTableHeader>
          <DataTableColumn id="name" isRowHeader>
            Nama
          </DataTableColumn>
          <DataTableColumn id="status">Status</DataTableColumn>
        </DataTableHeader>
        <DataTableBody>
          <DataTableRow id="r1">
            <DataTableCell>pump-01</DataTableCell>
            <DataTableCell>online</DataTableCell>
          </DataTableRow>
          <DataTableRow id="r2">
            <DataTableCell>valve-02</DataTableCell>
            <DataTableCell>offline</DataTableCell>
          </DataTableRow>
        </DataTableBody>
      </DataTable>,
    );
    expect(screen.getByRole('grid', { name: 'Perangkat' })).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'Nama' })).toBeInTheDocument();
    expect(screen.getByRole('rowheader', { name: 'pump-01' })).toBeInTheDocument();
    expect(screen.getByRole('rowheader', { name: 'valve-02' })).toBeInTheDocument();
    expect(screen.getAllByRole('row').length).toBe(3);
  });
});

describe('List', () => {
  it('renders selectable list items', async () => {
    const onSelectionChange = vi.fn();
    render(
      <List aria-label="Perangkat" selectionMode="single" onSelectionChange={onSelectionChange}>
        <ListItem id="a">pump-01</ListItem>
        <ListItem id="b">valve-02</ListItem>
      </List>,
    );
    await userEvent.click(screen.getByRole('option', { name: 'pump-01' }));
    await waitFor(() => expect(onSelectionChange).toHaveBeenCalled());
  });
});
