import type { ComponentProps, ReactNode } from 'react';
import {
  Alert,
  Button as HeroButton,
  Card,
  Chip,
  Description,
  FieldError,
  Input,
  Label,
  ListBox,
  Modal,
  NumberField,
  Select,
  Table,
  TextArea,
  TextField,
} from '@heroui/react';

export function Button({
  variant = 'secondary',
  size = 'md',
  ...props
}: ComponentProps<typeof HeroButton>) {
  return <HeroButton variant={variant} size={size} {...props} />;
}
export function Panel({
  title,
  description,
  actions,
  children,
  className = '',
}: {
  title?: ReactNode;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Card className={`cd-panel ${className}`}>
      {(title || actions) && (
        <Card.Header className="flex-row items-center justify-between gap-4">
          <div className="min-w-0">
            <Card.Title>{title}</Card.Title>
            {description && <Card.Description>{description}</Card.Description>}
          </div>
          {actions}
        </Card.Header>
      )}
      <Card.Content>{children}</Card.Content>
    </Card>
  );
}
const tones = {
  neutral: 'default',
  good: 'success',
  warn: 'warning',
  bad: 'danger',
  accent: 'accent',
} as const;
export function Badge({
  children,
  tone = 'neutral',
}: {
  children: ReactNode;
  tone?: keyof typeof tones;
}) {
  return (
    <Chip size="sm" color={tones[tone]} variant="soft">
      <Chip.Label>{children}</Chip.Label>
    </Chip>
  );
}
export function Notice({
  children,
  title,
  tone = 'neutral',
}: {
  children: ReactNode;
  title?: string;
  tone?: keyof typeof tones;
}) {
  return (
    <Alert status={tones[tone]} className="cd-notice">
      <Alert.Indicator />
      <Alert.Content>
        {title && <Alert.Title>{title}</Alert.Title>}
        <Alert.Description>{children}</Alert.Description>
      </Alert.Content>
    </Alert>
  );
}
export function Dialog({
  title,
  children,
  close,
  locked = false,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
  locked?: boolean;
}) {
  return (
    <Modal.Backdrop
      isOpen
      onOpenChange={(open) => {
        if (!open) close();
      }}
      isDismissable={!locked}
      isKeyboardDismissDisabled={locked}
      variant="blur"
    >
      <Modal.Container scroll="inside" size="lg">
        <Modal.Dialog className="cd-dialog">
          {!locked && <Modal.CloseTrigger aria-label="关闭对话框" />}
          <Modal.Header>
            <Modal.Heading>{title}</Modal.Heading>
          </Modal.Header>
          {children}
        </Modal.Dialog>
      </Modal.Container>
    </Modal.Backdrop>
  );
}
export function TextInput({
  label,
  help,
  multiline = false,
  rows = 3,
  placeholder,
  autoFocus,
  ...props
}: ComponentProps<typeof TextField> & {
  label: string;
  help?: string;
  multiline?: boolean;
  rows?: number;
  placeholder?: string;
}) {
  return (
    <TextField {...props} autoFocus={autoFocus}>
      <Label>{label}</Label>
      {multiline ? (
        <TextArea rows={rows} placeholder={placeholder} />
      ) : (
        <Input placeholder={placeholder} />
      )}
      {help && <Description>{help}</Description>}
      <FieldError />
    </TextField>
  );
}
export function SelectInput({
  label,
  options,
  value,
  onChange,
  name,
  className = '',
}: {
  label: string;
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
  name?: string;
  className?: string;
}) {
  return (
    <Select
      name={name}
      className={className}
      value={value}
      onChange={(key) => {
        if (typeof key === 'string') onChange(key);
      }}
      selectionMode="single"
      isRequired
    >
      <Label>{label}</Label>
      <Select.Trigger>
        <Select.Value />
        <Select.Indicator />
      </Select.Trigger>
      <Select.Popover>
        <ListBox>
          {options.map((option) => (
            <ListBox.Item id={option.value} key={option.value} textValue={option.label}>
              {option.label}
              <ListBox.ItemIndicator />
            </ListBox.Item>
          ))}
        </ListBox>
      </Select.Popover>
    </Select>
  );
}
export function PortInput({
  label,
  value,
  onChange,
  name,
}: {
  label: string;
  value: number | null;
  onChange: (value: number | null) => void;
  name: string;
}) {
  return (
    <NumberField
      name={name}
      value={value ?? NaN}
      onChange={(value) => onChange(Number.isNaN(value) ? null : value)}
      minValue={1}
      maxValue={65535}
      isRequired
      formatOptions={{ useGrouping: false }}
    >
      <Label>{label}</Label>
      <NumberField.Group className="cd-port-control">
        <NumberField.DecrementButton aria-label={`减小${label}`} />
        <NumberField.Input />
        <NumberField.IncrementButton aria-label={`增加${label}`} />
      </NumberField.Group>
      <FieldError />
    </NumberField>
  );
}
export function DataTable({
  label,
  columns,
  rows,
}: {
  label: string;
  columns: string[];
  rows: { id: string; cells: ReactNode[] }[];
}) {
  return (
    <Table variant="secondary">
      <Table.ScrollContainer>
        <Table.Content aria-label={label} className="cd-data-table">
          <Table.Header>
            {columns.map((column, index) => (
              <Table.Column id={`column-${index}`} key={column} isRowHeader={index === 0}>
                {column}
              </Table.Column>
            ))}
          </Table.Header>
          <Table.Body>
            {rows.map((row) => (
              <Table.Row id={row.id} key={row.id}>
                {row.cells.map((cell, index) => (
                  <Table.Cell key={index}>{cell}</Table.Cell>
                ))}
              </Table.Row>
            ))}
          </Table.Body>
        </Table.Content>
      </Table.ScrollContainer>
    </Table>
  );
}
export function EmptyState({
  title,
  children,
  action,
}: {
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="cd-empty">
      <h2>{title}</h2>
      <p>{children}</p>
      {action}
    </div>
  );
}
