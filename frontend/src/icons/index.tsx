import type { ComponentType, SVGProps } from 'react';
import Close from '@material-symbols/svg-400/outlined/close.svg?react';
import Menu from '@material-symbols/svg-400/outlined/menu.svg?react';
import ChevronLeft from '@material-symbols/svg-400/outlined/chevron_left.svg?react';
import ChevronRight from '@material-symbols/svg-400/outlined/chevron_right.svg?react';
import ArrowUpward from '@material-symbols/svg-400/outlined/arrow_upward.svg?react';
import ArrowDropDown from '@material-symbols/svg-400/outlined/arrow_drop_down.svg?react';
import Check from '@material-symbols/svg-400/outlined/check.svg?react';
import ErrorIcon from '@material-symbols/svg-400/outlined/error.svg?react';
import Warning from '@material-symbols/svg-400/outlined/warning.svg?react';
import Info from '@material-symbols/svg-400/outlined/info.svg?react';
import CheckCircle from '@material-symbols/svg-400/outlined/check_circle.svg?react';
import CalendarMonth from '@material-symbols/svg-400/outlined/calendar_month.svg?react';
import Remove from '@material-symbols/svg-400/outlined/remove.svg?react';

export type IconComponent = ComponentType<SVGProps<SVGSVGElement>>;

/** Material Symbols Outlined (400, grade 0, 24 px), imported one by one so only used icons ship. */
export const icons = {
  close: Close,
  menu: Menu,
  chevronLeft: ChevronLeft,
  chevronRight: ChevronRight,
  arrowUpward: ArrowUpward,
  arrowDropDown: ArrowDropDown,
  check: Check,
  error: ErrorIcon,
  warning: Warning,
  info: Info,
  checkCircle: CheckCircle,
  calendarMonth: CalendarMonth,
  remove: Remove,
} satisfies Record<string, IconComponent>;

export function Icon({ icon: Svg, className }: { icon: IconComponent; className?: string }) {
  return (
    <Svg
      aria-hidden="true"
      focusable="false"
      width={24}
      height={24}
      fill="currentColor"
      className={className}
    />
  );
}
