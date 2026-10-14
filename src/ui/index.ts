// The primitives. Every control here has an accessible name, a real role,
// keyboard support and a stable test id contract (data-testid passes
// through), so the same UI is operable by a person and by an agent.
export { Button, buttonVariants, type ButtonProps } from "./Button";
export { IconButton, type IconButtonProps } from "./IconButton";
export { Spinner } from "./Spinner";
export { Kbd } from "./Kbd";
export { Chip, Badge, chipVariants, type ChipProps } from "./Chip";
export { StatusGlyph, agentStateLabel, type AgentState } from "./StatusGlyph";
export { SectionHeader } from "./SectionHeader";
export { Row, type RowProps } from "./Row";
export { EmptyState } from "./EmptyState";
export { Progress } from "./Progress";
export { Tooltip, TooltipProvider } from "./Tooltip";
export { Field, Input, Textarea } from "./Field";
export { Switch, Checkbox } from "./Switch";
export { Select, type SelectOption } from "./Select";
export { Tabs, type TabItem } from "./Tabs";
export { Popover } from "./Popover";
export { Dialog } from "./Dialog";
export { ConfirmDialog } from "./ConfirmDialog";
export { Sheet } from "./Sheet";
export { ToastHost, showToast, toastManager, type ToastTone } from "./Toast";
export { Command, type CommandItem } from "./Command";
export { DecisionCard, type DecisionAction } from "./DecisionCard";
export { BudgetRing } from "./BudgetRing";
