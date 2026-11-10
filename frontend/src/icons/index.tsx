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
import Visibility from '@material-symbols/svg-400/outlined/visibility.svg?react';
import Download from '@material-symbols/svg-400/outlined/download.svg?react';
import Refresh from '@material-symbols/svg-400/outlined/refresh.svg?react';
import Apps from '@material-symbols/svg-400/outlined/apps.svg?react';
import CloudDownload from '@material-symbols/svg-400/outlined/cloud_download.svg?react';
import CloudUpload from '@material-symbols/svg-400/outlined/cloud_upload.svg?react';
import DataUsage from '@material-symbols/svg-400/outlined/data_usage.svg?react';
import Dns from '@material-symbols/svg-400/outlined/dns.svg?react';
import FactCheck from '@material-symbols/svg-400/outlined/fact_check.svg?react';
import Key from '@material-symbols/svg-400/outlined/key.svg?react';
import PersonSearch from '@material-symbols/svg-400/outlined/person_search.svg?react';
import ReceiptLong from '@material-symbols/svg-400/outlined/receipt_long.svg?react';
import Settings from '@material-symbols/svg-400/outlined/settings.svg?react';

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
  visibility: Visibility,
  download: Download,
  refresh: Refresh,
  apps: Apps,
  cloudDownload: CloudDownload,
  cloudUpload: CloudUpload,
  dataUsage: DataUsage,
  dns: Dns,
  factCheck: FactCheck,
  key: Key,
  personSearch: PersonSearch,
  receiptLong: ReceiptLong,
  settings: Settings,
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
