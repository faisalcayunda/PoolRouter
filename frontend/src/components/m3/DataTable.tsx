import {
  Cell,
  Column,
  Row,
  Table as RacTable,
  TableBody,
  TableHeader,
  type CellProps,
  type ColumnProps,
  type RowProps,
  type TableBodyProps,
  type TableHeaderProps,
  type TableProps,
} from 'react-aria-components';
import { cx } from './cx';

type Class<T> = { className?: string } & Omit<T, 'className'>;

/** Data table (ekstensi lokal E-1) di atas RAC Table: header halus, pemisah tipis, tanpa zebra. */
export function DataTable({ className, ...props }: Class<TableProps>) {
  return <RacTable {...props} className={cx('w-full border-collapse text-left', className)} />;
}

export function DataTableHeader<T extends object>({
  className,
  ...props
}: Class<TableHeaderProps<T>>) {
  return (
    <TableHeader
      {...props}
      className={cx(
        'typescale-label-medium bg-surface-container-low text-on-surface-variant',
        className,
      )}
    />
  );
}

export function DataTableColumn({ className, ...props }: Class<ColumnProps>) {
  return (
    <Column
      {...props}
      className={cx(
        'm3-focus border-b border-outline-variant px-4 py-2.5',
        'data-[sortable]:cursor-pointer',
        className,
      )}
    />
  );
}

export function DataTableBody<T extends object>({ className, ...props }: Class<TableBodyProps<T>>) {
  return <TableBody {...props} className={className} />;
}

export function DataTableRow<T extends object>({ className, ...props }: Class<RowProps<T>>) {
  return (
    <Row
      {...props}
      className={cx(
        'm3-state border-b border-outline-variant text-on-surface last:border-b-0',
        'data-[hovered]:bg-surface-container-low data-[focused]:bg-on-surface/8',
        className,
      )}
    />
  );
}

export function DataTableCell({ className, ...props }: Class<CellProps>) {
  return (
    <Cell {...props} className={cx('typescale-body-medium px-4 py-3 align-middle', className)} />
  );
}
