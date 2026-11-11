import {
  Button,
  ComboBox as RacComboBox,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  type ComboBoxProps as RacComboBoxProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';
import { Icon, icons } from '../../icons';

export interface ComboBoxProps<T extends object> extends Omit<RacComboBoxProps<T>, 'className'> {
  className?: string;
  label?: ReactNode;
  items?: Iterable<T>;
  children: ReactNode;
}

/** M3 combo box: editable input + dropdown trigger + popover listbox. */
export function ComboBox<T extends object>({
  label,
  items,
  children,
  className,
  ...props
}: ComboBoxProps<T>) {
  return (
    <RacComboBox {...props} className={cx('flex w-64 flex-col gap-1', className)}>
      {label ? <Label className="typescale-label-large text-on-surface">{label}</Label> : null}
      <div className="flex h-10 items-center rounded-extra-small border border-outline bg-surface-container-lowest pr-1 focus-within:border-secondary focus-within:ring-3 focus-within:ring-secondary">
        <Input className="typescale-body-medium h-full min-w-0 flex-1 bg-transparent px-3 text-on-surface outline-none placeholder:text-on-surface-variant" />
        {/* Trigger label comes from RAC's own localized strings; inventoried in
            src/components/m3/README.md (Addendum A 5.4). */}
        <Button className="m3-state m3-focus flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-small text-on-surface-variant">
          <Icon icon={icons.arrowDropDown} />
        </Button>
      </div>
      <Popover className="min-w-(--trigger-width) rounded-medium border border-outline-variant bg-surface-container-lowest shadow-level2">
        <ListBox
          items={items}
          className="typescale-body-medium max-h-72 overflow-auto p-1 text-on-surface outline-none"
        >
          {children}
        </ListBox>
      </Popover>
    </RacComboBox>
  );
}

export function ComboBoxItem({
  children,
  id,
  textValue,
}: {
  children: ReactNode;
  id: string | number;
  textValue?: string;
}) {
  return (
    <ListBoxItem
      id={id}
      textValue={textValue}
      className={cx(
        'typescale-body-medium cursor-pointer rounded-small px-3 py-2 text-on-surface outline-none',
        'data-[focused]:bg-surface-container data-[selected]:font-semibold data-[focus-visible]:outline-solid data-[focus-visible]:outline-3 data-[focus-visible]:-outline-offset-3 data-[focus-visible]:outline-secondary',
      )}
    >
      {children}
    </ListBoxItem>
  );
}
