// Icon names the app has always used, now drawn by lucide-react (same 24 px
// grid, 2 px stroke, currentColor). New code imports lucide icons directly;
// this map only keeps the existing call sites and the IconName type stable.
import {
  Archive,
  Calendar,
  Check,
  CircleHelp,
  Clock,
  Command,
  FileText,
  Info,
  Key,
  Lightbulb,
  Lock,
  MessageSquare,
  Moon,
  PanelLeft,
  PanelRight,
  Plug,
  Puzzle,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  StickyNote,
  Sun,
  Users,
  X,
  type LucideIcon,
} from "lucide-react";

const ICONS = {
  puzzle: Puzzle,
  key: Key,
  lightbulb: Lightbulb,
  lock: Lock,
  "file-text": FileText,
  "message-square": MessageSquare,
  sun: Sun,
  moon: Moon,
  plug: Plug,
  server: Server,
  "panel-right": PanelRight,
  "panel-left": PanelLeft,
  sparkles: Sparkles,
  users: Users,
  calendar: Calendar,
  "sticky-note": StickyNote,
  clock: Clock,
  command: Command,
  "help-circle": CircleHelp,
  info: Info,
  x: X,
  check: Check,
  archive: Archive,
  "shield-check": ShieldCheck,
  sliders: SlidersHorizontal,
} satisfies Record<string, LucideIcon>;

export type IconName = keyof typeof ICONS;

interface Props {
  name: IconName;
  size?: number;
  className?: string;
  strokeWidth?: number;
}

export function Icon({ name, size = 18, className, strokeWidth = 2 }: Props) {
  const Glyph = ICONS[name];
  return <Glyph size={size} strokeWidth={strokeWidth} className={className} aria-hidden />;
}
