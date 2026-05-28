import React from 'react';
import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import * as ProgressPrimitive from '@radix-ui/react-progress';
import * as RadioGroupPrimitive from '@radix-ui/react-radio-group';
import * as SelectPrimitive from '@radix-ui/react-select';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import * as TabsPrimitive from '@radix-ui/react-tabs';
import * as TooltipPrimitive from '@radix-ui/react-tooltip';
import { createRoot } from 'react-dom/client';
import './ui.css';

type AnyRecord = Record<string, any>;
type NamePath = string | number | Array<string | number>;

export type CheckboxChangeEvent = {
  target: {
    checked: boolean;
    value?: any;
  };
};

type BaseProps = {
  className?: string;
  style?: React.CSSProperties;
  children?: any;
  [key: string]: any;
};

type ButtonProps = Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'type'> & {
  type?: 'primary' | 'default' | 'dashed' | 'link' | 'text';
  danger?: boolean;
  size?: 'small' | 'middle' | 'large';
  loading?: boolean;
  disabled?: boolean;
  icon?: React.ReactNode;
  htmlType?: 'button' | 'submit' | 'reset';
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
  block?: boolean;
  ghost?: boolean;
  shape?: string;
};

type FormItemChildFunction = (helpers: Pick<FormStore, 'getFieldValue' | 'setFieldsValue' | 'setFieldValue'>) => React.ReactNode;
type FormRule = {
  required?: boolean;
  message?: React.ReactNode;
  validator?: (rule: FormRule, value: any) => Promise<void> | void;
  pattern?: RegExp;
  max?: number;
  min?: number;
  len?: number;
  whitespace?: boolean;
  [key: string]: any;
};
type FormShouldUpdate = ((previousValues: AnyRecord, currentValues: AnyRecord) => boolean) | boolean;

type InputProps = Omit<React.InputHTMLAttributes<HTMLInputElement>, 'size' | 'prefix'> & {
  status?: 'error' | 'warning';
  size?: 'small' | 'middle' | 'large';
  allowClear?: boolean;
  addonAfter?: React.ReactNode;
  addonBefore?: React.ReactNode;
  prefix?: React.ReactNode;
  suffix?: React.ReactNode;
  onPressEnter?: React.KeyboardEventHandler<HTMLInputElement>;
  visibilityToggle?: boolean;
  variant?: string;
  showTime?: boolean;
};

type TextAreaProps = Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, 'size'> & {
  autoSize?: boolean | AnyRecord;
  showCount?: boolean;
};

type InputNumberProps = Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'size'> & {
  value?: number | null;
  min?: number;
  max?: number;
  step?: number;
  onChange?: (value: number | null) => void;
  addonAfter?: React.ReactNode;
  addonBefore?: React.ReactNode;
  size?: 'small' | 'middle' | 'large';
  controls?: boolean;
  precision?: number;
};

type SelectOptionItem = {
  value?: any;
  label?: any;
  disabled?: boolean;
  options?: SelectOptionItem[];
  [key: string]: any;
};

type SelectProps = BaseProps & {
  value?: any;
  defaultValue?: any;
  mode?: 'multiple' | string;
  options?: SelectOptionItem[];
  allowClear?: boolean;
  placeholder?: React.ReactNode;
  disabled?: boolean;
  loading?: boolean;
  tokenSeparators?: string[];
  variant?: string;
  onChange?: (value: any, option?: any) => void;
  filterOption?: boolean | ((inputValue: string, option?: any) => boolean);
  onSearch?: (value: string) => void;
  optionRender?: (option: any) => React.ReactNode;
  optionFilterProp?: string;
  showSearch?: boolean | AnyRecord;
};

type CheckboxProps = BaseProps & {
  checked?: boolean;
  defaultChecked?: boolean;
  disabled?: boolean;
  indeterminate?: boolean;
  value?: any;
  onChange?: (event: CheckboxChangeEvent) => void;
  onClick?: React.MouseEventHandler<HTMLLabelElement>;
};

type SwitchProps = BaseProps & {
  checked?: boolean;
  defaultChecked?: boolean;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
};

type RadioProps = BaseProps & {
  value?: any;
};

type RadioGroupProps = BaseProps & {
  value?: any;
  defaultValue?: any;
  options?: SelectOptionItem[];
  onChange?: (event: RadioChangeEvent) => void;
};

type RadioButtonProps = RadioProps & {
  checked?: boolean;
};

export type MenuProps = {
  items?: MenuItem[];
  onClick?: (info: { key: string; domEvent?: Event }) => void;
  [key: string]: any;
};

export type MenuItem = {
  key?: React.Key;
  label?: React.ReactNode;
  icon?: React.ReactNode;
  danger?: boolean;
  disabled?: boolean;
  type?: 'divider' | 'group';
  children?: MenuItem[];
  onClick?: (info: { key: string; domEvent?: Event }) => void;
};

export type ColumnsType<T = AnyRecord> = Array<ColumnType<T>>;
export type TableProps<T = AnyRecord> = {
  columns?: ColumnsType<T>;
  dataSource?: T[];
  rowKey?: keyof T | ((record: T) => React.Key);
  pagination?: false | AnyRecord;
  rowSelection?: AnyRecord;
  loading?: boolean;
  size?: 'small' | 'middle' | 'large';
  onRow?: (record: T, index?: number) => React.HTMLAttributes<HTMLTableRowElement>;
  className?: string;
  style?: React.CSSProperties;
  scroll?: AnyRecord;
  bordered?: boolean;
  locale?: AnyRecord;
  onChange?: (...args: any[]) => void;
  [key: string]: any;
};

export type ColumnType<T = AnyRecord> = {
  title?: React.ReactNode;
  dataIndex?: keyof T | string | Array<string | number>;
  key?: React.Key;
  width?: number | string;
  align?: 'left' | 'center' | 'right';
  render?: (value: any, record: T, index: number) => React.ReactNode;
  ellipsis?: boolean;
  fixed?: string | boolean;
  sorter?: any;
};

export type TabsProps = {
  items?: TabItem[];
  activeKey?: string;
  defaultActiveKey?: string;
  onChange?: (activeKey: string) => void;
  onTabClick?: (activeKey: string) => void;
  className?: string;
  style?: React.CSSProperties;
  destroyInactiveTabPane?: boolean;
  tabBarExtraContent?: React.ReactNode | { left?: React.ReactNode; right?: React.ReactNode };
  size?: string;
  tabBarGutter?: number;
  indicator?: {
    size?: number | ((origin: number) => number);
    align?: string;
  } & AnyRecord;
  [key: string]: any;
};

type TabItem = {
  key: string;
  label: React.ReactNode;
  children?: React.ReactNode;
  disabled?: boolean;
  icon?: React.ReactNode;
};

export type RadioChangeEvent = {
  target: {
    value: any;
    checked?: boolean;
  };
};

const cx = (...classes: Array<string | false | null | undefined>) => classes.filter(Boolean).join(' ');

const toPath = (name: NamePath): Array<string | number> => Array.isArray(name) ? name : [name];

const pathKey = (name: NamePath) => toPath(name).join('.');

const isSamePath = (left: Array<string | number>, right: Array<string | number>) =>
  left.length === right.length && left.every((part, index) => part === right[index]);

const isPrefixPath = (prefix: Array<string | number>, path: Array<string | number>) =>
  prefix.length <= path.length && prefix.every((part, index) => part === path[index]);

const resolveNamePath = (name: NamePath | undefined, prefix: Array<string | number>) => {
  if (name === undefined) return undefined;
  const namePath = toPath(name);
  if (!prefix.length || isPrefixPath(prefix, namePath)) return namePath;
  return [...prefix, ...namePath];
};

const getIn = (source: AnyRecord, name: NamePath) => {
  let current: any = source;
  for (const part of toPath(name)) {
    if (current == null) return undefined;
    current = current[part];
  }
  return current;
};

const setIn = (source: AnyRecord, name: NamePath, value: any) => {
  const path = toPath(name);
  const next = Array.isArray(source) ? [...source] : { ...source };
  let current: any = next;
  path.forEach((part, index) => {
    if (index === path.length - 1) {
      current[part] = value;
      return;
    }
    const existing = current[part];
    const child = Array.isArray(existing)
      ? [...existing]
      : existing && typeof existing === 'object'
        ? { ...existing }
        : typeof path[index + 1] === 'number'
          ? []
          : {};
    current[part] = child;
    current = child;
  });
  return next;
};

const valueFromEvent = (event: any) => {
  if (event && event.target) {
    const target = event.target;
    if ('checked' in target && (target.type === 'checkbox' || target.type === undefined)) return target.checked;
    return target.value;
  }
  return event;
};

const validationError = (rule: FormRule) => new Error(String(rule.message || 'Validation failed'));

const isEmptyFormValue = (value: any) => {
  const isEmptyArray = Array.isArray(value) && value.length === 0;
  return value === undefined || value === null || value === '' || isEmptyArray;
};

const getComparableSize = (value: any) => {
  if (typeof value === 'string' || Array.isArray(value)) return value.length;
  if (typeof value === 'number') return value;
  return undefined;
};

const validateBuiltInRule = (rule: FormRule, value: any) => {
  if (rule.required && isEmptyFormValue(value)) {
    throw validationError(rule);
  }
  if (isEmptyFormValue(value)) {
    return;
  }
  if (rule.whitespace && typeof value === 'string' && value.length > 0 && value.trim().length === 0) {
    throw validationError(rule);
  }
  if (rule.pattern instanceof RegExp && typeof value === 'string') {
    rule.pattern.lastIndex = 0;
    if (!rule.pattern.test(value)) {
      throw validationError(rule);
    }
  }
  const comparableSize = getComparableSize(value);
  if (comparableSize === undefined) {
    return;
  }
  if (typeof rule.len === 'number' && comparableSize !== rule.len) {
    throw validationError(rule);
  }
  if (typeof rule.min === 'number' && comparableSize < rule.min) {
    throw validationError(rule);
  }
  if (typeof rule.max === 'number' && comparableSize > rule.max) {
    throw validationError(rule);
  }
};

type RegisteredRules = {
  namePath: Array<string | number>;
  rules: FormRule[];
};

class FormStore {
  private values: AnyRecord = {};
  private initialValues: AnyRecord = {};
  private listeners = new Set<() => void>();
  private submitHandler?: (values: any) => void;
  private valuesChangeHandler?: (changed: any, all: any) => void;
  private rules = new Map<string, RegisteredRules>();

  getFieldsValue = (_names?: NamePath[] | true) => this.values;
  getFieldValue = (name: NamePath) => getIn(this.values, name);
  setInitialValues = (values?: AnyRecord) => {
    this.initialValues = values ? { ...values } : {};
    this.values = { ...this.initialValues, ...this.values };
    this.notify();
  };
  setCallbacks = (callbacks: { onFinish?: (values: any) => void; onValuesChange?: (changed: any, all: any) => void }) => {
    this.submitHandler = callbacks.onFinish;
    this.valuesChangeHandler = callbacks.onValuesChange;
  };
  setFieldsValue = (values: AnyRecord) => {
    this.values = mergeValues(this.values, values);
    this.notify();
  };
  setFieldValue = (name: NamePath, value: any) => {
    this.values = setIn(this.values, name, value);
    this.valuesChangeHandler?.(setIn({}, name, value), this.values);
    this.notify();
  };
  setFieldInitialValue = (name: NamePath, value: any) => {
    if (getIn(this.values, name) !== undefined) return;
    this.initialValues = setIn(this.initialValues, name, value);
    this.values = setIn(this.values, name, value);
    this.notify();
  };
  resetFields = () => {
    this.values = { ...this.initialValues };
    this.notify();
  };
  validateFields = async (names?: NamePath[]): Promise<any> => {
    const filterPaths = names?.map(toPath);
    for (const { namePath, rules } of this.rules.values()) {
      if (filterPaths && !filterPaths.some((filterPath) => isSamePath(filterPath, namePath) || isPrefixPath(filterPath, namePath))) {
        continue;
      }
      const value = getIn(this.values, namePath);
      for (const rule of rules) {
        validateBuiltInRule(rule, value);
        if (rule?.validator) {
          await rule.validator(rule, value);
        }
      }
    }
    return this.values;
  };
  submit = () => {
    this.validateFields()
      .then((values) => this.submitHandler?.(values))
      .catch(() => undefined);
  };
  registerRules = (name: NamePath | undefined, rules?: FormRule[]) => {
    if (!name) {
      return () => {};
    }
    const namePath = toPath(name);
    const key = pathKey(namePath);
    if (rules?.length) this.rules.set(key, { namePath, rules });
    return () => {
      this.rules.delete(key);
    };
  };
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private notify = () => {
    this.listeners.forEach((listener) => listener());
  };
}

export type FormInstance<T = any> = FormStore & {
  getFieldsValue: (names?: NamePath[] | true) => T;
  setFieldsValue: (values: Partial<T> | AnyRecord) => void;
  validateFields: (names?: NamePath[]) => Promise<any>;
};

const mergeValues = (base: AnyRecord, patch: AnyRecord): AnyRecord => {
  let next: AnyRecord = Array.isArray(base) ? [...base] : { ...base };
  Object.entries(patch).forEach(([key, value]) => {
    if (value && typeof value === 'object' && !Array.isArray(value) && base[key] && typeof base[key] === 'object' && !Array.isArray(base[key])) {
      next[key] = mergeValues(base[key], value);
    } else {
      next[key] = value;
    }
  });
  return next;
};

const FormContext = React.createContext<FormStore | null>(null);
const FormListPrefixContext = React.createContext<Array<string | number>>([]);

const useForceUpdate = () => {
  const [, setTick] = React.useState(0);
  return React.useCallback(() => setTick((tick) => tick + 1), []);
};

const useFormStore = (provided?: FormStore) => {
  const ref = React.useRef<FormStore | null>(null);
  if (!ref.current) ref.current = provided || new FormStore();
  return provided || ref.current;
};

const createControlChild = (child: React.ReactNode, props: AnyRecord) => {
  if (!React.isValidElement(child)) return child;
  return React.cloneElement(child as React.ReactElement<any>, props);
};

type FormProps<T = AnyRecord> = BaseProps & {
  form?: FormInstance<T>;
  initialValues?: Partial<T>;
  layout?: 'horizontal' | 'vertical' | 'inline';
  onFinish?: (values: T) => void;
  onValuesChange?: (changedValues: Partial<T>, values: T) => void;
};

type FormItemProps = BaseProps & {
  name?: NamePath;
  label?: React.ReactNode;
  initialValue?: any;
  valuePropName?: string;
  noStyle?: boolean;
  required?: boolean;
  rules?: FormRule[];
  extra?: React.ReactNode;
  shouldUpdate?: FormShouldUpdate;
  wrapperCol?: AnyRecord;
  labelCol?: AnyRecord;
  getValueFromEvent?: (...args: any[]) => any;
  children?: any;
};

type FormItemComponent = {
  (props: Omit<FormItemProps, 'children' | 'shouldUpdate'> & { shouldUpdate: FormShouldUpdate; children: FormItemChildFunction }): React.ReactElement | null;
  (props: FormItemProps): React.ReactElement | null;
};

type FormListOperation = {
  add: (defaultValue?: any) => void;
  remove: (index: number | number[]) => void;
};

type FormListField = {
  key: React.Key;
  name: number;
  fieldKey?: number;
};

type FormListProps = {
  name: NamePath;
  children: (fields: FormListField[], operations: FormListOperation) => React.ReactNode;
};

const FormComponent = <T extends AnyRecord = AnyRecord>({
  form,
  initialValues,
  onFinish,
  onValuesChange,
  layout = 'horizontal',
  className,
  children,
  ...rest
}: FormProps<T>) => {
  const store = useFormStore(form);
  React.useEffect(() => {
    store.setInitialValues(initialValues);
  }, [store, initialValues]);
  React.useEffect(() => {
    store.setCallbacks({ onFinish, onValuesChange });
  }, [store, onFinish, onValuesChange]);
  return (
    <FormContext.Provider value={store}>
      <form
        {...rest}
        className={cx('ant-form', layout && `ant-form-${layout}`, 'ui-form', `ui-form-${layout}`, className)}
        onSubmit={(event) => {
          event.preventDefault();
          store.submit();
        }}
      >
        {children}
      </form>
    </FormContext.Provider>
  );
};

const FormItem = ({
  name,
  label,
  initialValue,
  children,
  valuePropName = 'value',
  noStyle,
  required,
  rules,
  extra,
  help,
  hidden,
  className,
  style,
  shouldUpdate,
  getValueFromEvent,
  ...rest
}: FormItemProps) => {
  const store = React.useContext(FormContext);
  const listPrefix = React.useContext(FormListPrefixContext);
  const resolvedName = React.useMemo(() => resolveNamePath(name, listPrefix), [name, listPrefix]);
  const forceUpdate = useForceUpdate();
  React.useEffect(() => store?.subscribe(forceUpdate), [store, forceUpdate]);
  React.useEffect(() => {
    if (!store || resolvedName === undefined || initialValue === undefined) return undefined;
    store.setFieldInitialValue(resolvedName, initialValue);
    return undefined;
  }, [store, resolvedName, initialValue]);
  React.useEffect(() => store?.registerRules(resolvedName, rules || (required ? [{ required: true }] : undefined)), [store, resolvedName, rules, required]);

  if (shouldUpdate && typeof children === 'function') {
    const helperStore = store || new FormStore();
    return (
      <div className={cx(noStyle ? undefined : 'ant-form-item ui-form-item', className)} style={style}>
        {children({
          getFieldValue: helperStore.getFieldValue,
          setFieldsValue: helperStore.setFieldsValue,
          setFieldValue: helperStore.setFieldValue,
        })}
      </div>
    );
  }

  const value = resolvedName && store ? store.getFieldValue(resolvedName) : undefined;
  const controlProps = resolvedName && store
    ? {
        [valuePropName]: valuePropName === 'checked' ? Boolean(value) : value,
        onChange: (...args: any[]) => {
          store.setFieldValue(resolvedName, getValueFromEvent ? getValueFromEvent(...args) : valueFromEvent(args[0]));
          const originalOnChange = React.isValidElement(children) ? (children.props as AnyRecord).onChange : undefined;
          originalOnChange?.(...args);
        },
      }
    : {};
  const control = createControlChild(children, controlProps);

  if (noStyle) return <>{control}</>;
  return (
    <div {...rest} hidden={hidden} className={cx('ant-form-item', 'ui-form-item', className)} style={style}>
      {label && <div className="ant-form-item-label ui-form-item-label"><label className="ui-form-label">{label}{required && <span className="ui-required">*</span>}</label></div>}
      <div className="ant-form-item-control ui-form-control">
        <div className="ant-form-item-control-input">{control}</div>
        {help && <div className="ui-form-help">{help}</div>}
        {extra && <div className="ui-form-extra">{extra}</div>}
      </div>
    </div>
  );
};

const FormList = ({ name, children }: FormListProps) => {
  const store = React.useContext(FormContext);
  const listPrefix = React.useContext(FormListPrefixContext);
  const resolvedName = React.useMemo(() => resolveNamePath(name, listPrefix) || toPath(name), [name, listPrefix]);
  const forceUpdate = useForceUpdate();
  React.useEffect(() => store?.subscribe(forceUpdate), [store, forceUpdate]);
  const values = (store?.getFieldValue(resolvedName) || []) as any[];
  const fields = values.map((_, index) => ({ key: index, name: index }));
  const operations = {
    add: (value?: any) => store?.setFieldValue(resolvedName, [...values, value ?? undefined]),
    remove: (index: number | number[]) => {
      const indexes = Array.isArray(index) ? index : [index];
      store?.setFieldValue(resolvedName, values.filter((_, itemIndex) => !indexes.includes(itemIndex)));
    },
  };
  return (
    <FormListPrefixContext.Provider value={resolvedName}>
      {children(fields, operations)}
    </FormListPrefixContext.Provider>
  );
};

type FormNamespace = (<T extends AnyRecord = AnyRecord>(props: FormProps<T>) => React.ReactElement) & {
  Item: FormItemComponent;
  List: React.FC<FormListProps>;
  useForm: <T extends AnyRecord = any>() => [FormInstance<T>];
  useWatch: (name: NamePath, options?: { form?: FormInstance; preserve?: boolean } | FormInstance) => any;
};

(FormComponent as FormNamespace).Item = FormItem as FormItemComponent;
(FormComponent as FormNamespace).List = FormList;
(FormComponent as FormNamespace).useForm = <T extends AnyRecord = any>() => {
  const formRef = React.useRef<FormStore | null>(null);
  if (!formRef.current) formRef.current = new FormStore();
  return [formRef.current as FormInstance<T>];
};
(FormComponent as FormNamespace).useWatch = (name: NamePath, options?: { form?: FormInstance; preserve?: boolean } | FormInstance) => {
  const contextStore = React.useContext(FormContext);
  const explicitStore = options instanceof FormStore ? options : options?.form;
  const store = explicitStore || contextStore;
  const forceUpdate = useForceUpdate();
  React.useEffect(() => store?.subscribe(forceUpdate), [store, forceUpdate]);
  return store?.getFieldValue(name);
};

export const Form = FormComponent as FormNamespace;

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(({
  type,
  danger,
  size,
  loading,
  disabled,
  icon,
  children,
  className,
  htmlType,
  block,
  ghost,
  shape,
  ...rest
}, ref) => (
  <button
    {...rest}
    ref={ref}
    type={htmlType || 'button'}
    disabled={disabled || loading}
    className={cx('ant-btn', type === 'primary' && 'ant-btn-primary', (!type || type === 'default' || type === 'dashed') && 'ant-btn-default', type === 'dashed' && 'ant-btn-dashed', type === 'link' && 'ant-btn-link', type === 'text' && 'ant-btn-text', danger && 'ant-btn-dangerous', 'ui-btn', type === 'primary' && 'ui-btn-primary', type === 'link' && 'ui-btn-link', type === 'text' && 'ui-btn-text', type === 'dashed' && 'ui-btn-dashed', danger && 'ui-btn-danger', size === 'small' && 'ui-btn-sm', block && 'ui-btn-block', ghost && 'ui-btn-ghost', shape === 'circle' && 'ui-btn-circle', className)}
  >
    {loading && <span className="ui-spinner ui-spinner-inline" />}
    {icon}
    {children}
  </button>
));
Button.displayName = 'Button';

type SpaceProps = Omit<React.HTMLAttributes<HTMLDivElement>, 'size'> & {
  direction?: 'horizontal' | 'vertical';
  orientation?: 'horizontal' | 'vertical' | string;
  size?: number | 'small' | 'middle' | 'large' | [number, number];
  wrap?: boolean;
  align?: React.CSSProperties['alignItems'];
};

type SpaceComponentType = React.FC<SpaceProps> & {
  Compact: React.FC<SpaceProps>;
};

const getSpaceGap = (size: SpaceProps['size']) => {
  if (Array.isArray(size)) return size[0];
  if (typeof size === 'number') return size;
  if (size === 'large') return 16;
  if (size === 'small') return 4;
  return 8;
};

const SpaceComponent: SpaceComponentType = ({ children, className, direction, orientation, size = 8, wrap, align, style, ...rest }) => {
  const resolvedDirection = direction || orientation || 'horizontal';
  return (
  <div {...rest} className={cx('ui-space', resolvedDirection === 'vertical' && 'ui-space-vertical', wrap && 'ui-space-wrap', className)} style={{ gap: getSpaceGap(size), alignItems: align, ...style }}>
    {children}
  </div>
  );
};
SpaceComponent.Compact = ({ children, className, ...rest }) => <div {...rest} className={cx('ui-space ui-space-compact', className)}>{children}</div>;
export const Space = SpaceComponent;

type TypographyTextProps = BaseProps & {
  type?: string;
  strong?: boolean;
  code?: boolean;
  copyable?: any;
  ellipsis?: any;
};

type TypographyTitleProps = BaseProps & {
  level?: 1 | 2 | 3 | 4 | 5;
};

type TypographyParagraphProps = BaseProps & {
  type?: string;
  copyable?: any;
  ellipsis?: any;
};

type TypographyLinkProps = Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'type'> & {
  type?: string;
  href?: string;
  target?: string;
};

export const Typography: {
  Text: React.FC<TypographyTextProps>;
  Title: React.FC<TypographyTitleProps>;
  Paragraph: React.FC<TypographyParagraphProps>;
  Link: React.FC<TypographyLinkProps>;
} = {
  Text: ({ type, strong, code, copyable: _copyable, ellipsis: _ellipsis, className, children, ...rest }: AnyRecord) => {
    const TagName = code ? 'code' : 'span';
    return <TagName {...rest} className={cx('ui-typography-text', type && `ui-text-${type}`, strong && 'ui-text-strong', className)}>{children}</TagName>;
  },
  Title: ({ level = 1, className, children, ...rest }: AnyRecord) => {
    const TagName = `h${level}` as React.ElementType;
    return <TagName {...rest} className={cx('ui-title', `ui-title-${level}`, className)}>{children}</TagName>;
  },
  Paragraph: ({ type, className, children, ...rest }: AnyRecord) => <p {...rest} className={cx('ui-paragraph', type && `ui-text-${type}`, className)}>{children}</p>,
  Link: ({ className, children, onClick, ...rest }: AnyRecord) => <button type="button" {...rest} onClick={onClick as React.MouseEventHandler<HTMLButtonElement>} className={cx('ui-link', className)}>{children}</button>,
};

const TextInput = React.forwardRef<HTMLInputElement, InputProps>(({
  className,
  status,
  addonAfter,
  addonBefore,
  prefix,
  suffix,
  allowClear,
  size: _size,
  visibilityToggle: _visibilityToggle,
  variant,
  onPressEnter,
  onKeyDown,
  onChange,
  value,
  ...rest
}, ref) => {
  const needsAffix = Boolean(prefix || suffix || addonBefore || addonAfter || allowClear);
  const inputElement = (
    <input
      {...rest}
      ref={ref}
      value={value}
      className={cx('ant-input', 'ui-input', variant === 'borderless' && 'ui-input-borderless', status === 'error' && 'ui-input-error', needsAffix ? 'ui-input-composed' : undefined, className)}
      onChange={onChange}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onPressEnter?.(event);
        onKeyDown?.(event);
      }}
    />
  );
  const input = needsAffix ? (
    <span className="ant-input-affix-wrapper ui-input-affix">
      {prefix && <span className="ui-input-prefix">{prefix}</span>}
      {inputElement}
      {allowClear && value ? (
        <button
          type="button"
          className="ui-input-clear"
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            const event = { target: { value: '' }, currentTarget: { value: '' } } as unknown as React.ChangeEvent<HTMLInputElement>;
            onChange?.(event);
          }}
        >
          ×
        </button>
      ) : null}
      {suffix && <span className="ui-input-suffix">{suffix}</span>}
    </span>
  ) : inputElement;
  if (!prefix && !suffix && !addonBefore && !addonAfter) return input;
  return (
    <span className="ui-input-group">
      {addonBefore && <span className="ant-input-group-addon ui-input-addon">{addonBefore}</span>}
      {input}
      {addonAfter && <span className="ant-input-group-addon ui-input-addon">{addonAfter}</span>}
    </span>
  );
});
TextInput.displayName = 'Input';

const TextArea = React.forwardRef<HTMLTextAreaElement, TextAreaProps>(({ className, autoSize: _autoSize, showCount: _showCount, ...rest }, ref) => (
  <textarea {...rest} ref={ref} className={cx('ant-input', 'ui-input ui-textarea', className)} />
));
TextArea.displayName = 'Input.TextArea';

const Password = React.forwardRef<HTMLInputElement, InputProps>(({ suffix, visibilityToggle = true, ...props }, ref) => {
  const [visible, setVisible] = React.useState(false);
  return (
    <TextInput
      {...props}
      ref={ref}
      type={visible ? 'text' : 'password'}
      suffix={visibilityToggle === false ? suffix : (
        <>
          {suffix}
          <button type="button" className="ui-input-clear" onMouseDown={(event) => event.preventDefault()} onClick={() => setVisible((current) => !current)}>
            {visible ? 'Hide' : 'Show'}
          </button>
        </>
      )}
    />
  );
});
Password.displayName = 'Input.Password';

type SearchProps = InputProps & {
  onSearch?: (value: string) => void;
};

const Search = React.forwardRef<HTMLInputElement, SearchProps>(({ onSearch, onPressEnter, ...props }, ref) => (
  <TextInput
    {...props}
    ref={ref}
    onKeyDown={(event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === 'Enter') onSearch?.((event.target as HTMLInputElement).value);
      onPressEnter?.(event);
    }}
  />
));
Search.displayName = 'Input.Search';

type InputComponent = React.ForwardRefExoticComponent<InputProps & React.RefAttributes<HTMLInputElement>> & {
  TextArea: React.ForwardRefExoticComponent<TextAreaProps & React.RefAttributes<HTMLTextAreaElement>>;
  Password: React.ForwardRefExoticComponent<InputProps & React.RefAttributes<HTMLInputElement>>;
  Search: React.ForwardRefExoticComponent<SearchProps & React.RefAttributes<HTMLInputElement>>;
};

export const Input = Object.assign(TextInput, { TextArea, Password, Search }) as InputComponent;

export const InputNumber = React.forwardRef<HTMLInputElement, InputNumberProps>(({ value, onChange, min, max, step, className, addonAfter, addonBefore, size: _size, controls: _controls, precision: _precision, ...rest }, ref) => {
  const input = (
    <input
      {...rest}
      ref={ref}
      type="number"
      min={min}
      max={max}
      step={step}
      value={value ?? ''}
      className={cx('ant-input-number ant-input-number-input', 'ui-input ui-input-number', (addonBefore || addonAfter) ? 'ui-input-composed' : undefined, className)}
      onChange={(event) => onChange?.(event.target.value === '' ? null : Number(event.target.value))}
    />
  );
  if (!addonBefore && !addonAfter) return input;
  return (
    <span className="ui-input-group">
      {addonBefore && <span className="ant-input-group-addon ui-input-addon">{addonBefore}</span>}
      {input}
      {addonAfter && <span className="ant-input-group-addon ui-input-addon">{addonAfter}</span>}
    </span>
  );
});
InputNumber.displayName = 'InputNumber';

const normalizeOptions = (options?: any[], children?: React.ReactNode) => {
  const result: Array<{ value: any; label: React.ReactNode; disabled?: boolean }> = [];
  options?.forEach((option) => {
    if (option?.options) {
      option.options.forEach((child: any) => result.push({ value: child.value, label: child.label ?? child.value, disabled: child.disabled }));
    } else if (option?.value !== undefined) {
      result.push({ value: option.value, label: option.label ?? option.value, disabled: option.disabled });
    } else {
      result.push({ value: option.label, label: option.label, disabled: option.disabled });
    }
  });
  React.Children.forEach(children, (child) => {
    if (React.isValidElement(child)) {
      const props = child.props as AnyRecord;
      if (props.value !== undefined) result.push({ value: props.value, label: props.children ?? props.label ?? props.value, disabled: props.disabled });
    }
  });
  return result;
};

const SelectOption = ({ children }: AnyRecord) => <>{children}</>;

const optionText = (value: React.ReactNode): string => {
  if (typeof value === 'string' || typeof value === 'number') return String(value);
  if (Array.isArray(value)) return value.map(optionText).join(' ');
  if (React.isValidElement(value)) return optionText((value.props as AnyRecord).children);
  return '';
};

const getSearchOptionFilterProp = (showSearch: SelectProps['showSearch'], optionFilterProp?: string) => {
  if (showSearch && typeof showSearch === 'object') {
    return showSearch.optionFilterProp ?? optionFilterProp;
  }
  return optionFilterProp;
};

const optionMatchesSearch = (
  inputValue: string,
  option: SelectOptionItem,
  filterOption: SelectProps['filterOption'],
  optionFilterProp?: string,
) => {
  if (!inputValue || filterOption === false) return true;
  if (typeof filterOption === 'function') return filterOption(inputValue, option);
  const searchTarget = optionFilterProp
    ? option[optionFilterProp]
    : option.label ?? option.value;
  return optionText(searchTarget).toLowerCase().includes(inputValue.toLowerCase());
};

const SearchableSingleSelect = ({
  selectedValue,
  emitChange,
  normalized,
  allowClear,
  placeholder,
  disabled,
  className,
  style,
  variant,
  filterOption,
  optionFilterProp,
  showSearch,
  onSearch,
  optionRender,
}: SelectProps & {
  selectedValue: any;
  emitChange: (nextValue: any, option?: any) => void;
  normalized: SelectOptionItem[];
}) => {
  const [open, setOpen] = React.useState(false);
  const [searchValue, setSearchValue] = React.useState('');
  const selectedOption = normalized.find((option) => String(option.value) === String(selectedValue));
  const effectiveOptionFilterProp = getSearchOptionFilterProp(showSearch, optionFilterProp);
  const filteredOptions = normalized.filter((option) => optionMatchesSearch(searchValue, option, filterOption, effectiveOptionFilterProp));
  const selectedLabel = selectedOption?.label ?? selectedOption?.value;
  const hasSelection = selectedValue !== undefined && selectedValue !== null && selectedValue !== '';
  const updateSearchValue = (nextValue: string) => {
    setSearchValue(nextValue);
    onSearch?.(nextValue);
  };
  const clearSearchValue = () => setSearchValue('');
  const selectOption = (option: SelectOptionItem) => {
    emitChange(option.value, option);
    setOpen(false);
    clearSearchValue();
  };

  return (
    <PopoverPrimitive.Root
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) clearSearchValue();
      }}
    >
      <PopoverPrimitive.Trigger asChild>
        <button
          type="button"
          disabled={disabled}
          className={cx('ant-select-selector', 'ui-select-trigger', variant === 'borderless' && 'ui-select-borderless', className)}
          style={style}
        >
          <span className={cx(!hasSelection && 'ui-select-placeholder')}>
            {hasSelection ? selectedLabel : placeholder}
          </span>
        </button>
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content className="ant-select-dropdown ui-select-content ui-select-searchable-content" align="start" sideOffset={4}>
          <input
            className="ant-select-selection-search-input ui-select-search-input"
            autoFocus
            value={searchValue}
            placeholder={typeof placeholder === 'string' ? placeholder : undefined}
            onChange={(event) => updateSearchValue(event.target.value)}
          />
          {allowClear && hasSelection && (
            <button
              type="button"
              className="ant-select-item ant-select-item-option ui-select-item"
              onClick={() => {
                emitChange(undefined, undefined);
                setOpen(false);
                clearSearchValue();
              }}
            >
              -
            </button>
          )}
          {filteredOptions.map((option) => (
            <button
              type="button"
              key={String(option.value)}
              disabled={option.disabled}
              className={cx('ant-select-item ant-select-item-option ui-select-item', String(option.value) === String(selectedValue) && 'ui-select-item-selected')}
              onClick={() => selectOption(option)}
            >
              {optionRender ? optionRender(option) : option.label}
            </button>
          ))}
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
};

const TagsSelect = ({ value, defaultValue, onChange, options, children, placeholder, disabled, className, style, tokenSeparators = [','], variant, allowClear, ...rest }: SelectProps) => {
  const inputId = React.useId();
  const [inputValue, setInputValue] = React.useState('');
  const normalized = normalizeOptions(options, children);
  const separators: string[] = Array.isArray(tokenSeparators) ? tokenSeparators : [','];
  const isControlled = value !== undefined;
  const [innerValues, setInnerValues] = React.useState<any[]>(Array.isArray(defaultValue) ? defaultValue : []);
  const selectedValues = isControlled
    ? (Array.isArray(value) ? value : [])
    : innerValues;
  const updateValues = (next: any[]) => {
    if (!isControlled) setInnerValues(next);
    onChange?.(next, normalized.filter((option) => next.includes(option.value) || next.includes(String(option.value))));
  };
  const addValues = (rawValue: string) => {
    const parts = separators.reduce<string[]>((items, separator) => items.flatMap((item) => item.split(separator)), [rawValue])
      .map((item) => item.trim())
      .filter(Boolean);
    if (!parts.length) return;
    const next = Array.from(new Set([...selectedValues, ...parts]));
    updateValues(next);
    setInputValue('');
  };
  const removeValue = (removedValue: any) => {
    const next = selectedValues.filter((item: any) => item !== removedValue);
    updateValues(next);
  };
  return (
    <div className={cx('ant-select-selector', 'ui-select-tags', variant === 'borderless' && 'ui-select-borderless', disabled && 'ui-disabled', className)} style={style}>
      {selectedValues.map((item: any) => (
        <span className="ui-select-tag" key={String(item)}>
          {String(item)}
          <button type="button" disabled={disabled} onClick={() => removeValue(item)}>×</button>
        </span>
      ))}
      <input
        {...rest}
        list={inputId}
        disabled={disabled}
        className="ui-select-tags-input"
        value={inputValue}
        placeholder={selectedValues.length ? undefined : placeholder as string | undefined}
        onChange={(event) => setInputValue(event.target.value)}
        onBlur={() => addValues(inputValue)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || separators.includes(event.key)) {
            event.preventDefault();
            addValues(inputValue);
          }
          if (event.key === 'Backspace' && !inputValue && selectedValues.length) {
            removeValue(selectedValues[selectedValues.length - 1]);
          }
        }}
      />
      {allowClear && selectedValues.length > 0 && (
        <button type="button" className="ui-select-clear" disabled={disabled} onClick={() => updateValues([])}>×</button>
      )}
      <datalist id={inputId}>
        {normalized.map((option) => <option key={String(option.value)} value={String(option.value)} />)}
      </datalist>
    </div>
  );
};

const SelectComponent = ({
  value,
  defaultValue,
  onChange,
  options,
  children,
  mode,
  allowClear,
  placeholder,
  disabled,
  className,
  style,
  variant,
  showSearch,
  filterOption,
  optionFilterProp,
  onSearch,
  optionRender,
  ...rest
}: SelectProps) => {
  const normalized = normalizeOptions(options, children);
  const isControlled = value !== undefined;
  const [innerValue, setInnerValue] = React.useState(defaultValue ?? (mode === 'multiple' ? [] : ''));
  const selectedValues = isControlled ? value : innerValue;
  const emitChange = (nextValue: any, option?: any) => {
    if (!isControlled) setInnerValue(nextValue);
    onChange?.(nextValue, option);
  };
  if (mode === 'tags') {
    return <TagsSelect value={value} defaultValue={defaultValue} onChange={onChange} options={options} disabled={disabled} placeholder={placeholder} className={className} style={style} variant={variant} allowClear={allowClear} {...rest}>{children}</TagsSelect>;
  }
  if (mode === 'multiple') {
    const selectedMultipleValues = Array.isArray(selectedValues) ? selectedValues : [];
    return (
      <select
        {...rest}
        multiple
        disabled={disabled}
        value={selectedMultipleValues}
        className={cx('ant-select-selector', 'ui-select-native', variant === 'borderless' && 'ui-select-borderless', className)}
        style={style}
        onChange={(event) => {
          const next = Array.from(event.target.selectedOptions).map((option) => option.value);
          emitChange(next, normalized.filter((option) => next.includes(String(option.value))));
        }}
      >
        {normalized.map((option) => <option key={String(option.value)} value={option.value} disabled={option.disabled}>{option.label}</option>)}
      </select>
    );
  }
  if (showSearch || filterOption || optionFilterProp) {
    return (
      <SearchableSingleSelect
        selectedValue={selectedValues}
        emitChange={emitChange}
        normalized={normalized}
        allowClear={allowClear}
        placeholder={placeholder}
        disabled={disabled}
        className={className}
        style={style}
        variant={variant}
        showSearch={showSearch}
        filterOption={filterOption}
        optionFilterProp={optionFilterProp}
        onSearch={onSearch}
        optionRender={optionRender}
        {...rest}
      />
    );
  }
  return (
    <SelectPrimitive.Root
      value={selectedValues === undefined || selectedValues === null ? '' : String(selectedValues)}
      disabled={disabled}
      onValueChange={(nextValue) => {
        if (nextValue === '__clear__') {
          emitChange(undefined, undefined);
          return;
        }
        const option = normalized.find((item) => String(item.value) === nextValue);
        emitChange(option?.value ?? nextValue, option);
      }}
    >
      <SelectPrimitive.Trigger className={cx('ant-select-selector', 'ui-select-trigger', variant === 'borderless' && 'ui-select-borderless', className)} style={style}>
        <SelectPrimitive.Value placeholder={placeholder} />
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content className="ant-select-dropdown ui-select-content" position="popper">
          <SelectPrimitive.Viewport>
            {allowClear && <SelectPrimitive.Item className="ant-select-item ant-select-item-option ui-select-item" value="__clear__">-</SelectPrimitive.Item>}
            {normalized.map((option) => (
              <SelectPrimitive.Item key={String(option.value)} value={String(option.value)} disabled={option.disabled} className="ant-select-item ant-select-item-option ui-select-item">
                <SelectPrimitive.ItemText>{option.label}</SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
};
SelectComponent.Option = SelectOption;
export const Select = SelectComponent as typeof SelectComponent & { Option: typeof SelectOption };

const AutoCompleteComponent = ({
  value,
  defaultValue,
  onChange,
  options,
  children,
  placeholder,
  disabled,
  className,
  style,
  filterOption,
  allowClear: _allowClear,
  onSearch,
  variant,
  ...rest
}: SelectProps) => {
  const listId = React.useId();
  const normalized = normalizeOptions(options, children);
  const isControlled = value !== undefined;
  const [innerValue, setInnerValue] = React.useState(defaultValue ?? '');
  const currentValue = isControlled ? value : innerValue;
  const filteredOptions = typeof filterOption === 'function' && typeof currentValue === 'string'
    ? normalized.filter((option) => filterOption(currentValue, option))
    : normalized;
  return (
    <>
      <input
        {...rest}
        list={listId}
        disabled={disabled}
        value={currentValue}
        placeholder={placeholder as string | undefined}
        className={cx('ant-input', 'ui-input ui-autocomplete', variant === 'borderless' && 'ui-input-borderless', className)}
        style={style}
        onChange={(event) => {
          const nextValue = event.target.value;
          if (!isControlled) setInnerValue(nextValue);
          const option = normalized.find((item) => String(item.value) === nextValue);
          onChange?.(nextValue, option);
          onSearch?.(nextValue);
        }}
      />
      <datalist id={listId}>
        {filteredOptions.map((option) => (
          <option key={String(option.value)} value={String(option.value)} label={optionText(option.label)} />
        ))}
      </datalist>
    </>
  );
};
AutoCompleteComponent.Option = SelectOption;
export const AutoComplete = AutoCompleteComponent as typeof AutoCompleteComponent & { Option: typeof SelectOption };

const CheckboxComponent = ({ checked, defaultChecked, indeterminate, onChange, onClick, children, disabled, className, value, ...rest }: CheckboxProps) => {
  const [innerChecked, setInnerChecked] = React.useState(Boolean(defaultChecked));
  const isChecked = checked ?? innerChecked;
  return (
    <label className={cx('ant-checkbox-wrapper', 'ui-checkbox-wrapper', disabled && 'ui-disabled', className)} onClick={onClick}>
      <CheckboxPrimitive.Root
        {...rest}
        disabled={disabled}
        checked={isChecked}
        className="ant-checkbox ui-checkbox"
        data-indeterminate={indeterminate ? 'true' : undefined}
        onCheckedChange={(nextChecked) => {
          const nextValue = nextChecked === true;
          setInnerChecked(nextValue);
          onChange?.({ target: { checked: nextValue, value, type: 'checkbox' } as any });
        }}
      >
        <CheckboxPrimitive.Indicator>{indeterminate ? '-' : '✓'}</CheckboxPrimitive.Indicator>
      </CheckboxPrimitive.Root>
      {children && <span>{children}</span>}
    </label>
  );
};
CheckboxComponent.Group = ({ options = [], value = [], onChange, children, className }: AnyRecord) => (
  <div className={cx('ui-checkbox-group', className)}>
    {options.map((option: any) => (
      <Checkbox
        key={String(option.value)}
        checked={value.includes(option.value)}
        onChange={(event: any) => {
          const next = event.target.checked ? [...value, option.value] : value.filter((item: any) => item !== option.value);
          onChange?.(next);
        }}
      >
        {option.label}
      </Checkbox>
    ))}
    {children}
  </div>
);
type CheckboxComponentType = React.FC<CheckboxProps> & {
  Group: React.FC<{
    options?: SelectOptionItem[];
    value?: any[];
    onChange?: (checkedValue: any[]) => void;
    className?: string;
    children?: React.ReactNode;
  }>;
};
export const Checkbox = CheckboxComponent as CheckboxComponentType;

const RadioComponent = ({ value, children, className, ...rest }: RadioProps) => (
  <RadioGroupPrimitive.Item {...rest} value={String(value)} className={cx('ant-radio-wrapper', 'ui-radio', className)}>
    <span className="ui-radio-indicator" />
    <span>{children}</span>
  </RadioGroupPrimitive.Item>
);
RadioComponent.Group = ({ value, defaultValue, onChange, options, children, className }: RadioGroupProps) => (
  <RadioGroupPrimitive.Root
    value={value ?? defaultValue}
    className={cx('ant-radio-group', 'ui-radio-group', className)}
    onValueChange={(nextValue) => onChange?.({ target: { value: nextValue } })}
  >
    {options?.map((option: any) => <Radio key={String(option.value)} value={option.value}>{option.label}</Radio>)}
    {children}
  </RadioGroupPrimitive.Root>
);
RadioComponent.Button = ({ value, children, className, ...rest }: RadioButtonProps) => (
  <RadioGroupPrimitive.Item {...rest} value={String(value)} className={cx('ui-radio-button', className)}>
    {children}
  </RadioGroupPrimitive.Item>
);
type RadioComponentType = React.FC<RadioProps> & {
  Group: React.FC<RadioGroupProps>;
  Button: React.FC<RadioButtonProps>;
};
export const Radio = RadioComponent as RadioComponentType;

export const Switch = ({ checked, defaultChecked, onChange, disabled, className, ...rest }: SwitchProps) => {
  const [innerChecked, setInnerChecked] = React.useState(Boolean(defaultChecked));
  const isChecked = checked ?? innerChecked;
  return (
    <SwitchPrimitive.Root
      {...rest}
      disabled={disabled}
      checked={isChecked}
      className={cx('ui-switch', className)}
      onCheckedChange={(nextChecked) => {
        setInnerChecked(nextChecked);
        onChange?.(nextChecked);
      }}
    >
      <SwitchPrimitive.Thumb className="ui-switch-thumb" />
    </SwitchPrimitive.Root>
  );
};

type TagProps = React.HTMLAttributes<HTMLSpanElement> & {
  color?: string;
  bordered?: boolean;
  icon?: React.ReactNode;
  closable?: boolean;
  onClose?: (event: React.MouseEvent<HTMLButtonElement>) => void;
  closeIcon?: React.ReactNode;
  variant?: string;
};

export const Tag: React.FC<TagProps> = ({ color, bordered: _bordered, icon, closable, onClose, closeIcon, variant: _variant, className, children, ...rest }) => (
  <span {...rest} className={cx('ant-tag', 'ui-tag', color && `ui-tag-${color}`, className)}>
    {icon}{children}
    {closable && <button type="button" className="ui-tag-close" onClick={onClose}>{closeIcon || '×'}</button>}
  </span>
);

type CardProps = Omit<React.HTMLAttributes<HTMLElement>, 'title'> & {
  title?: React.ReactNode;
  extra?: React.ReactNode;
  bodyStyle?: React.CSSProperties;
  styles?: { body?: React.CSSProperties };
  size?: string;
};

export const Card: React.FC<CardProps> = ({ title, extra, children, className, bodyStyle, styles, ...rest }) => (
  <section {...rest} className={cx('ant-card', 'ui-card', className)}>
    {(title || extra) && <div className="ant-card-head ui-card-head"><div className="ant-card-head-title ui-card-title">{title}</div><div className="ant-card-extra">{extra}</div></div>}
    <div className="ant-card-body ui-card-body" style={{ ...bodyStyle, ...styles?.body }}>{children}</div>
  </section>
);

export const Empty = ({ description, image, children, className, ...rest }: AnyRecord) => (
  <div {...rest} className={cx('ui-empty', className)}>
    {image}
    <div>{description || 'No data'}</div>
    {children}
  </div>
);
Empty.PRESENTED_IMAGE_SIMPLE = null;

export const Spin = ({ spinning = true, children, className, ...rest }: AnyRecord) => (
  <div {...rest} className={cx('ant-spin-nested-loading', 'ui-spin-nested-loading', className)}>
    {spinning && <span className="ui-spinner" />}
    <div className="ant-spin-container ui-spin-container">{children}</div>
  </div>
);

export const Alert = ({ type = 'info', message, title, description, showIcon, icon, closable, onClose, className, action, closeIcon, ...rest }: AnyRecord) => {
  const [closed, setClosed] = React.useState(false);
  if (closed) return null;
  return (
    <div {...rest} className={cx('ant-alert', 'ui-alert', `ui-alert-${type}`, className)}>
      {showIcon && <span className="ui-alert-icon">{icon || '!'}</span>}
      <div className="ui-alert-content">
        {(title || message) && <div className="ui-alert-message">{title || message}</div>}
        {description && <div className="ui-alert-description">{description}</div>}
      </div>
      {action}
      {closable && (
        <button
          type="button"
          className="ui-alert-close"
          onClick={(event) => {
            setClosed(true);
            onClose?.(event);
          }}
        >
          {closeIcon || '×'}
        </button>
      )}
    </div>
  );
};

export const Divider = ({ children, className }: AnyRecord) => <div className={cx('ui-divider', className)}>{children && <span>{children}</span>}</div>;

export const Tooltip = ({ title, children, className }: AnyRecord) => (
  <TooltipPrimitive.Provider>
    <TooltipPrimitive.Root delayDuration={220}>
      <TooltipPrimitive.Trigger asChild>{React.isValidElement(children) ? children : <span>{children}</span>}</TooltipPrimitive.Trigger>
      {title && (
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Content className={cx('ui-tooltip', className)} sideOffset={6}>{title}</TooltipPrimitive.Content>
        </TooltipPrimitive.Portal>
      )}
    </TooltipPrimitive.Root>
  </TooltipPrimitive.Provider>
);

type DropdownProps = BaseProps & {
  menu?: MenuProps;
  trigger?: string[];
  overlayClassName?: string;
};

export const Dropdown: React.FC<DropdownProps> = ({ menu, children, trigger: _trigger, overlayClassName }) => (
  <DropdownMenuPrimitive.Root>
    <DropdownMenuPrimitive.Trigger asChild>{React.isValidElement(children) ? children : <button type="button">{children}</button>}</DropdownMenuPrimitive.Trigger>
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content className={cx('ant-dropdown-menu', 'ui-dropdown-content', overlayClassName)} align="end" sideOffset={6}>
        {menu?.items?.map((item: MenuItem) => {
          if (!item) return null;
          if (item.type === 'divider') return <DropdownMenuPrimitive.Separator key={String(item.key ?? Math.random())} className="ui-dropdown-separator" />;
          return (
            <DropdownMenuPrimitive.Item
              key={String(item.key)}
              disabled={item.disabled}
              className={cx('ant-dropdown-menu-item', 'ui-dropdown-item', item.danger && 'ui-dropdown-danger')}
              onSelect={(event) => {
                item.onClick?.({ key: String(item.key), domEvent: event as unknown as Event });
                menu?.onClick?.({ key: String(item.key), domEvent: event as unknown as Event });
              }}
            >
              {item.icon}
              {item.label}
            </DropdownMenuPrimitive.Item>
          );
        })}
      </DropdownMenuPrimitive.Content>
    </DropdownMenuPrimitive.Portal>
  </DropdownMenuPrimitive.Root>
);

type ModalFooterRender = (
  originNode: React.ReactNode,
  extra: {
    OkBtn: React.FC<ButtonProps>;
    CancelBtn: React.FC<ButtonProps>;
  }
) => React.ReactNode;

type ModalProps = BaseProps & {
  open?: boolean;
  visible?: boolean;
  title?: React.ReactNode;
  footer?: React.ReactNode | ModalFooterRender;
  onOk?: () => void;
  onCancel?: () => void;
  okText?: React.ReactNode;
  cancelText?: React.ReactNode;
  okButtonProps?: ButtonProps;
  cancelButtonProps?: ButtonProps;
  confirmLoading?: boolean;
  closable?: boolean;
  centered?: boolean;
  width?: number | string;
  maskClosable?: boolean;
  keyboard?: boolean;
};

type ModalStaticConfig = {
  title?: React.ReactNode;
  content?: React.ReactNode;
  onOk?: () => void;
  onCancel?: () => void;
  footer?: React.ReactNode | ModalFooterRender;
  okText?: React.ReactNode;
  cancelText?: React.ReactNode;
  okType?: 'primary' | 'danger' | 'default';
  okButtonProps?: ButtonProps;
  cancelButtonProps?: ButtonProps;
  closable?: boolean;
  maskClosable?: boolean;
  keyboard?: boolean;
  [key: string]: any;
};

type ModalComponent = React.FC<ModalProps> & {
  confirm: (config: ModalStaticConfig) => { destroy: () => void };
  error: (config: ModalStaticConfig) => { destroy: () => void } | void;
  info: (config: ModalStaticConfig) => { destroy: () => void } | void;
  success: (config: ModalStaticConfig) => { destroy: () => void } | void;
  warning: (config: ModalStaticConfig) => { destroy: () => void };
  destroyAll: () => void;
};

const ModalComponent: ModalComponent = ({
  open,
  visible,
  title,
  children,
  footer,
  onOk,
  onCancel,
  okText = 'OK',
  cancelText = 'Cancel',
  okButtonProps,
  cancelButtonProps,
  confirmLoading,
  closable = true,
  centered: _centered,
  className,
  width,
  maskClosable = true,
  keyboard = true,
  destroyOnClose: _destroyOnClose,
  destroyOnHidden: _destroyOnHidden,
  ...rest
}) => {
  const isOpen = Boolean(open ?? visible);
  const handleCancel = () => {
    if (confirmLoading) return;
    onCancel?.();
  };
  const OkBtn: React.FC<ButtonProps> = (buttonProps = {}) => (
    <Button
      type="primary"
      loading={confirmLoading}
      onClick={onOk}
      {...okButtonProps}
      {...buttonProps}
    >
      {okText}
    </Button>
  );
  const CancelBtn: React.FC<ButtonProps> = (buttonProps = {}) => (
    <Button
      onClick={handleCancel}
      {...cancelButtonProps}
      {...buttonProps}
    >
      {cancelText}
    </Button>
  );
  const defaultFooter = (
    <div className="ant-modal-footer ui-modal-footer">
      <CancelBtn />
      <OkBtn />
    </div>
  );
  const resolvedFooter = footer === null
    ? null
    : typeof footer === 'function'
      ? footer(defaultFooter, { OkBtn, CancelBtn })
      : footer ?? defaultFooter;
  return (
    <DialogPrimitive.Root open={isOpen} onOpenChange={(nextOpen) => !nextOpen && handleCancel()}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="ui-modal-mask" />
        <DialogPrimitive.Content
          {...rest}
          className={cx('ant-modal-wrap', 'ui-modal-wrap', className)}
          style={{ width, maxWidth: width, ...rest.style }}
          onPointerDownOutside={(event) => {
            if (!maskClosable) event.preventDefault();
          }}
          onEscapeKeyDown={(event) => {
            if (!keyboard) event.preventDefault();
          }}
        >
          <div className="ant-modal ui-modal">
            <div className="ant-modal-container ui-modal-container">
              <div className="ant-modal-content ui-modal-content">
                <div className="ant-modal-header ui-modal-header">
                  <DialogPrimitive.Title className="ant-modal-title ui-modal-title">{title}</DialogPrimitive.Title>
                  {closable && <DialogPrimitive.Close asChild><button type="button" className="ant-modal-close ui-modal-close">×</button></DialogPrimitive.Close>}
                </div>
                <div className="ant-modal-body ui-modal-body">{children}</div>
                {resolvedFooter}
              </div>
            </div>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
};

const staticModalDestroyers = new Set<() => void>();

const showStaticModal = (config: ModalStaticConfig, mode: 'confirm' | 'notice' = 'confirm') => {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  let destroyed = false;
  const destroy = () => {
    if (destroyed) return;
    destroyed = true;
    root.unmount();
    container.remove();
    staticModalDestroyers.delete(destroy);
  };
  staticModalDestroyers.add(destroy);

  const handleOk = async () => {
    await config.onOk?.();
    destroy();
  };
  const handleCancel = () => {
    config.onCancel?.();
    destroy();
  };

  const OkBtn: React.FC<ButtonProps> = (buttonProps = {}) => {
    const { onClick, ...restButtonProps } = buttonProps;
    return (
      <Button
        type={config.okType === 'danger' ? 'default' : 'primary'}
        danger={config.okType === 'danger'}
        onClick={async (event) => {
          onClick?.(event);
          if (!event.defaultPrevented) await handleOk();
        }}
        {...config.okButtonProps}
        {...restButtonProps}
      >
        {config.okText || 'OK'}
      </Button>
    );
  };
  const CancelBtn: React.FC<ButtonProps> = (buttonProps = {}) => {
    const { onClick, ...restButtonProps } = buttonProps;
    return (
      <Button
        onClick={(event) => {
          onClick?.(event);
          if (!event.defaultPrevented) handleCancel();
        }}
        {...config.cancelButtonProps}
        {...restButtonProps}
      >
        {config.cancelText || 'Cancel'}
      </Button>
    );
  };
  const defaultFooter = mode === 'confirm'
    ? (
      <div className="ant-modal-footer ui-modal-footer">
        <CancelBtn />
        <OkBtn />
      </div>
    )
    : (
      <div className="ant-modal-footer ui-modal-footer">
        <OkBtn />
      </div>
    );
  const footer = config.footer === null
    ? null
    : typeof config.footer === 'function'
      ? config.footer(defaultFooter, { OkBtn, CancelBtn })
      : config.footer ?? defaultFooter;

  root.render(
    <Modal
      open
      title={config.title}
      footer={footer}
      onCancel={handleCancel}
      closable={config.closable ?? true}
      maskClosable={config.maskClosable ?? true}
      keyboard={config.keyboard ?? true}
      width={config.width}
    >
      {config.content}
    </Modal>
  );
  return { destroy };
};
ModalComponent.confirm = (config: ModalStaticConfig) => showStaticModal(config, 'confirm');
ModalComponent.error = (config: ModalStaticConfig) => showStaticModal(config, 'notice');
ModalComponent.info = ModalComponent.error;
ModalComponent.success = ModalComponent.error;
ModalComponent.warning = (config: ModalStaticConfig) => showStaticModal(config, 'notice');
ModalComponent.destroyAll = () => staticModalDestroyers.forEach((destroy) => destroy());
export const Modal = ModalComponent;

type PopconfirmProps = BaseProps & {
  title?: React.ReactNode;
  description?: React.ReactNode;
  disabled?: boolean;
  onConfirm?: (event?: React.MouseEvent) => void;
  onCancel?: (event?: React.MouseEvent) => void;
  okText?: React.ReactNode;
  cancelText?: React.ReactNode;
  okButtonProps?: ButtonProps;
  cancelButtonProps?: ButtonProps;
};

export const Popconfirm: React.FC<PopconfirmProps> = ({
  title,
  description,
  onConfirm,
  onCancel,
  okText = 'OK',
  cancelText = 'Cancel',
  okButtonProps,
  cancelButtonProps,
  disabled,
  children,
}) => {
  const [open, setOpen] = React.useState(false);
  if (disabled) return <>{children}</>;
  return (
    <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
      <PopoverPrimitive.Trigger asChild>{React.isValidElement(children) ? children : <button type="button">{children}</button>}</PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content className="ui-popconfirm" sideOffset={8} align="center">
          <div className="ui-popconfirm-title">{title}</div>
          {description && <div className="ui-popconfirm-description">{description}</div>}
          <div className="ui-popconfirm-actions">
            <Button
              size="small"
              onClick={(event) => {
                onCancel?.(event);
                setOpen(false);
              }}
              {...cancelButtonProps}
            >
              {cancelText}
            </Button>
            <Button
              size="small"
              type="primary"
              onClick={(event) => {
                onConfirm?.(event);
                setOpen(false);
              }}
              {...okButtonProps}
            >
              {okText}
            </Button>
          </div>
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
};

type CollapseKey = string;
type CollapseProps = BaseProps & {
  items?: Array<AnyRecord>;
  defaultActiveKey?: CollapseKey | CollapseKey[];
  activeKey?: CollapseKey | CollapseKey[];
  onChange?: (keys: CollapseKey[]) => void;
  bordered?: boolean;
  ghost?: boolean;
};

type CollapseComponent = React.FC<CollapseProps> & {
  Panel: React.FC<AnyRecord>;
};

const CollapseComponent: CollapseComponent = ({ items, children, className, defaultActiveKey, activeKey, onChange, ...rest }) => {
  const normalized = items || React.Children.toArray(children).map((child: any) => child?.props).filter(Boolean);
  const initial = Array.isArray(defaultActiveKey) ? defaultActiveKey[0] : defaultActiveKey;
  const activeKeys = activeKey === undefined ? undefined : (Array.isArray(activeKey) ? activeKey.map(String) : [String(activeKey)]);
  return (
    <div {...rest} className={cx('ant-collapse ui-collapse', className)}>
      {normalized.map((item: AnyRecord) => (
        <details
          key={String(item.key)}
          className="ant-collapse-item ui-collapse-item"
          open={(activeKeys ? activeKeys.includes(String(item.key)) : undefined) ?? (initial ? String(initial) === String(item.key) : undefined)}
          onToggle={(event) => {
            const isOpen = (event.target as HTMLDetailsElement).open;
            const key = String(item.key);
            const baseKeys = activeKeys ?? [];
            onChange?.(isOpen ? Array.from(new Set([...baseKeys, key])) : baseKeys.filter((activeItemKey) => activeItemKey !== key));
          }}
        >
          <summary className="ant-collapse-header ui-collapse-header"><span className="ant-collapse-expand-icon ui-collapse-expand-icon" />{item.label || item.header}</summary>
          <div className="ant-collapse-content ui-collapse-content">
            <div className="ant-collapse-content-box ui-collapse-content-box">{item.children}</div>
          </div>
        </details>
      ))}
    </div>
  );
};
CollapseComponent.Panel = ({ children }: AnyRecord) => <>{children}</>;
export const Collapse = CollapseComponent;

export const Tabs = ({ items = [], activeKey, defaultActiveKey, onChange, onTabClick, className, style, tabBarExtraContent }: TabsProps) => {
  const firstKey = items[0]?.key;
  const [innerActiveKey, setInnerActiveKey] = React.useState(defaultActiveKey ?? firstKey);
  const extraContent = tabBarExtraContent && typeof tabBarExtraContent === 'object' && !React.isValidElement(tabBarExtraContent) && !Array.isArray(tabBarExtraContent)
    ? tabBarExtraContent as { left?: React.ReactNode; right?: React.ReactNode }
    : null;
  const leftExtra = extraContent
    ? extraContent.left
    : null;
  const rightExtra = extraContent
    ? extraContent.right
    : tabBarExtraContent;
  const currentActiveKey = activeKey !== undefined ? activeKey : innerActiveKey;
  const handleValueChange = (nextKey: string) => {
    if (activeKey === undefined) setInnerActiveKey(nextKey);
    onChange?.(nextKey);
  };
  const tabsProps = activeKey !== undefined
    ? { value: activeKey }
    : { defaultValue: innerActiveKey };
  return (
    <TabsPrimitive.Root {...tabsProps} onValueChange={handleValueChange} className={cx('ant-tabs ui-tabs', className)} style={style}>
      <div className="ui-tabs-nav-row">
        {leftExtra}
        <TabsPrimitive.List className="ant-tabs-nav ui-tabs-list">
          {items.map((item) => (
            <TabsPrimitive.Trigger
              key={item.key}
              value={item.key}
              disabled={item.disabled}
              className={cx('ant-tabs-tab ui-tabs-trigger', currentActiveKey === item.key && 'ant-tabs-tab-active')}
              onClick={() => onTabClick?.(item.key)}
            >
              <span className="ant-tabs-tab-btn">{item.icon}{item.label}</span>
            </TabsPrimitive.Trigger>
          ))}
          <span className="ant-tabs-ink-bar ui-tabs-ink-bar" />
        </TabsPrimitive.List>
        {rightExtra && <>{rightExtra}</>}
      </div>
      {items.map((item) => <TabsPrimitive.Content key={item.key} value={item.key} className="ui-tabs-content">{item.children}</TabsPrimitive.Content>)}
    </TabsPrimitive.Root>
  );
};

type SegmentedOption = string | number | {
  value: string | number;
  label?: React.ReactNode;
  disabled?: boolean;
};

type SegmentedProps = BaseProps & {
  options?: SegmentedOption[];
  value?: string | number;
  onChange?: (value: string | number) => void;
  size?: 'small' | 'middle' | 'large';
};

export const Segmented: React.FC<SegmentedProps> = ({ options = [], value, onChange, className }) => (
  <div className={cx('ant-segmented-group', 'ui-segmented', className)}>
    {options.map((option) => {
      const itemValue = typeof option === 'object' ? option.value : option;
      const label = typeof option === 'object' ? option.label : option;
      const disabled = typeof option === 'object' ? option.disabled : false;
      return <button key={String(itemValue)} type="button" disabled={disabled} className={cx('ant-segmented-item', 'ui-segmented-item', value === itemValue && 'ui-segmented-active')} onClick={() => onChange?.(itemValue)}><span className="ant-segmented-item-label">{label}</span></button>;
    })}
  </div>
);

export const Table = <T extends AnyRecord = AnyRecord>({ columns = [], dataSource = [], rowKey, rowSelection, loading, className, style, scroll, pagination, locale, bordered, onRow }: TableProps<T>) => {
  const paginationConfig = pagination === false ? undefined : pagination;
  const [currentPage, setCurrentPage] = React.useState(paginationConfig?.current ?? 1);
  const pageSize = paginationConfig?.pageSize ?? (dataSource.length || 1);
  const shouldPaginate = Boolean(paginationConfig);
  const visibleData = shouldPaginate
    ? dataSource.slice((currentPage - 1) * pageSize, currentPage * pageSize)
    : dataSource;
  const selectedKeys = rowSelection?.selectedRowKeys || [];
  const selectableRows = rowSelection
    ? visibleData
        .map((record, index) => ({ record, index, key: getRowKey(record, index, rowKey), checkboxProps: rowSelection.getCheckboxProps?.(record) || {} }))
        .filter((row) => !row.checkboxProps.disabled)
    : [];
  const selectableKeys = selectableRows.map((row) => row.key);
  const selectedSelectableCount = selectableKeys.filter((key) => selectedKeys.includes(key)).length;
  const allSelectableChecked = selectableKeys.length > 0 && selectedSelectableCount === selectableKeys.length;
  const partiallyChecked = selectedSelectableCount > 0 && selectedSelectableCount < selectableKeys.length;
  return (
    <div className={cx('ui-table-wrap', bordered && 'ui-table-bordered', className)} style={style}>
      {loading && <Spin />}
      <table className="ant-table ui-table" style={{ minWidth: scroll?.x }}>
        <thead className="ant-table-thead">
          <tr>
            {rowSelection && (
              <th>
                <Checkbox
                  checked={allSelectableChecked}
                  indeterminate={partiallyChecked}
                  disabled={selectableKeys.length === 0}
                  onChange={(event: any) => {
                    const nextKeys = event.target.checked ? selectableKeys : [];
                    const nextRows = event.target.checked ? selectableRows.map((row) => row.record) : [];
                    rowSelection.onChange?.(nextKeys, nextRows);
                  }}
                />
              </th>
            )}
            {columns.map((column, index) => <th key={String(column.key ?? column.dataIndex ?? index)} style={{ width: column.width, textAlign: column.align }}>{column.title}</th>)}
          </tr>
        </thead>
        <tbody className="ant-table-tbody">
          {visibleData.map((record, rowIndex) => {
            const key = getRowKey(record, rowIndex, rowKey);
            const checkboxProps = rowSelection?.getCheckboxProps?.(record) || {};
            return (
              <tr key={String(key)} {...onRow?.(record, rowIndex)}>
                {rowSelection && <td><Checkbox {...checkboxProps} checked={selectedKeys.includes(key)} onChange={(event: any) => {
                  const selected = new Set(selectedKeys);
                  if (event.target.checked) selected.add(key);
                  else selected.delete(key);
                  const keys = Array.from(selected);
                  rowSelection.onChange?.(keys, dataSource.filter((item, index) => keys.includes(getRowKey(item, index, rowKey))));
                }} /></td>}
                {columns.map((column, columnIndex) => {
                  const value = column.dataIndex ? getIn(record as AnyRecord, column.dataIndex as NamePath) : undefined;
                  return <td key={String(column.key ?? column.dataIndex ?? columnIndex)} style={{ textAlign: column.align }}>{column.render ? column.render(value, record, rowIndex) : value}</td>;
                })}
              </tr>
            );
          })}
          {visibleData.length === 0 && (
            <tr>
              <td className="ui-table-empty" colSpan={columns.length + (rowSelection ? 1 : 0)}>
                {locale?.emptyText || <Empty description="No data" />}
              </td>
            </tr>
          )}
        </tbody>
      </table>
      {shouldPaginate && dataSource.length > pageSize && (
        <div className="ui-table-pagination">
          <Pagination
            current={currentPage}
            total={dataSource.length}
            pageSize={pageSize}
            onChange={(page, nextPageSize) => {
              setCurrentPage(page);
              paginationConfig?.onChange?.(page, nextPageSize);
            }}
          />
        </div>
      )}
    </div>
  );
};

const getRowKey = <T extends AnyRecord>(record: T, index: number, rowKey?: keyof T | ((record: T) => React.Key)) => {
  if (typeof rowKey === 'function') return rowKey(record);
  if (rowKey) return record[rowKey] as React.Key;
  return (record.key ?? record.id ?? index) as React.Key;
};

export const Progress = ({ percent = 0, status: _status, strokeColor, className }: AnyRecord) => (
  <ProgressPrimitive.Root className={cx('ui-progress', className)} value={percent}>
    <ProgressPrimitive.Indicator className="ui-progress-indicator" style={{ transform: `translateX(-${100 - Number(percent)}%)`, background: strokeColor }} />
  </ProgressPrimitive.Root>
);

type DatePickerValue = string | DateLikeValue | null | undefined;
type RangePickerValue = [DatePickerValue, DatePickerValue] | null | undefined;
type DateLikeValue = {
  toDate: () => Date;
  format?: (format?: string) => string;
  valueOf?: () => number;
};
type DatePickerProps = BaseProps & {
  value?: DatePickerValue;
  onChange?: (date: DatePickerValue, dateString: string) => void;
  showTime?: boolean;
  variant?: string;
};
type RangePickerProps = BaseProps & {
  value?: RangePickerValue;
  onChange?: (dates: RangePickerValue, dateStrings: [string, string]) => void;
  showTime?: boolean;
  variant?: string;
};
type DatePickerComponent = React.FC<DatePickerProps> & {
  RangePicker: React.FC<RangePickerProps>;
};

const toInputDateString = (value: DatePickerValue, showTime?: boolean) => {
  if (!value) return '';
  if (typeof value === 'string') return showTime ? value.replace(' ', 'T').slice(0, 16) : value.slice(0, 10);
  const date = value.toDate();
  if (Number.isNaN(date.getTime())) return '';
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  if (!showTime) return `${year}-${month}-${day}`;
  const hour = String(date.getHours()).padStart(2, '0');
  const minute = String(date.getMinutes()).padStart(2, '0');
  return `${year}-${month}-${day}T${hour}:${minute}`;
};

const createDateLike = (value: string, showTime?: boolean): DateLikeValue | null => {
  if (!value) return null;
  const [datePart, timePart = '00:00'] = value.split('T');
  const [year = '0', month = '1', day = '1'] = datePart.split('-');
  const [hour = '0', minute = '0'] = timePart.split(':');
  const date = showTime
    ? new Date(Number(year), Number(month) - 1, Number(day), Number(hour), Number(minute))
    : new Date(Number(year), Number(month) - 1, Number(day));
  if (Number.isNaN(date.getTime())) return null;
  return {
    toDate: () => new Date(date),
    valueOf: () => date.getTime(),
    format: () => value,
  };
};

const DatePickerComponent: DatePickerComponent = ({ value, onChange, className, showTime, variant }: DatePickerProps) => (
  <input
    className={cx('ui-input', variant === 'borderless' && 'ui-input-borderless', className)}
    type={showTime ? 'datetime-local' : 'date'}
    value={toInputDateString(value, showTime)}
    onChange={(event) => onChange?.(createDateLike(event.target.value, showTime), event.target.value)}
  />
);
DatePickerComponent.RangePicker = ({ value, onChange, className, showTime, variant }: RangePickerProps) => (
  <div className={cx('ui-range-picker', className)}>
    <input
      className={cx('ui-input', variant === 'borderless' && 'ui-input-borderless')}
      type={showTime ? 'datetime-local' : 'date'}
      value={toInputDateString(value?.[0], showTime)}
      onChange={(event) => {
        const nextStart = createDateLike(event.target.value, showTime);
        const endString = toInputDateString(value?.[1], showTime);
        onChange?.([nextStart, value?.[1] || null], [event.target.value, endString]);
      }}
    />
    <span>-</span>
    <input
      className={cx('ui-input', variant === 'borderless' && 'ui-input-borderless')}
      type={showTime ? 'datetime-local' : 'date'}
      value={toInputDateString(value?.[1], showTime)}
      onChange={(event) => {
        const startString = toInputDateString(value?.[0], showTime);
        const nextEnd = createDateLike(event.target.value, showTime);
        onChange?.([value?.[0] || null, nextEnd], [startString, event.target.value]);
      }}
    />
  </div>
);
export const DatePicker = DatePickerComponent;

type PaginationProps = BaseProps & {
  current?: number;
  total?: number;
  pageSize?: number;
  onChange?: (page: number, pageSize: number) => void;
};

export const Pagination: React.FC<PaginationProps> = ({ current = 1, total = 0, pageSize = 10, onChange, className }) => (
  <div className={cx('ui-pagination', className)}>
    <Button size="small" disabled={current <= 1} onClick={() => onChange?.(current - 1, pageSize)}>Prev</Button>
    <span>{current} / {Math.max(1, Math.ceil(total / pageSize))}</span>
    <Button size="small" disabled={current >= Math.ceil(total / pageSize)} onClick={() => onChange?.(current + 1, pageSize)}>Next</Button>
  </div>
);

const DescriptionsComponent = ({ items, children, className }: AnyRecord) => (
  <dl className={cx('ui-descriptions', className)}>
    {items?.map((item: AnyRecord) => <React.Fragment key={String(item.key ?? item.label)}><dt>{item.label}</dt><dd>{item.children}</dd></React.Fragment>)}
    {children}
  </dl>
);
DescriptionsComponent.Item = ({ label, children }: AnyRecord) => <><dt>{label}</dt><dd>{children}</dd></>;
export const Descriptions = DescriptionsComponent;

type ListProps<T = any> = BaseProps & {
  dataSource?: T[];
  renderItem?: (item: T, index: number) => React.ReactNode;
};

type ListComponent = (<T = any>(props: ListProps<T>) => React.ReactElement) & {
  Item: React.FC<BaseProps & { actions?: React.ReactNode[] }> & {
    Meta: React.FC<{
      avatar?: React.ReactNode;
      title?: React.ReactNode;
      description?: React.ReactNode;
    }>;
  };
};

const ListComponent = <T = any,>({ dataSource = [], renderItem, children, className }: ListProps<T>) => (
  <div className={cx('ui-list', className)}>
    {dataSource.map((item, index) => renderItem ? renderItem(item, index) : item as React.ReactNode)}
    {children}
  </div>
);
const ListItem = (({ children, className, actions }: AnyRecord) => <div className={cx('ui-list-item', className)}><div>{children}</div>{actions && <div className="ui-list-actions">{actions}</div>}</div>) as ListComponent['Item'];
ListItem.Meta = ({ avatar, title, description }: { avatar?: React.ReactNode; title?: React.ReactNode; description?: React.ReactNode }) => (
  <div className="ui-list-meta">
    {avatar && <div className="ui-list-meta-avatar">{avatar}</div>}
    <div className="ui-list-meta-content">
      {title && <div className="ui-list-meta-title">{title}</div>}
      {description && <div className="ui-list-meta-description">{description}</div>}
    </div>
  </div>
);
ListComponent.Item = ListItem;
export const List = ListComponent as ListComponent;

const spanPercent = (span?: number) => span ? `${(span / 24) * 100}%` : undefined;

export const Row = ({ children, gutter, className, style, ...rest }: AnyRecord) => (
  <div {...rest} className={cx('ui-row', className)} style={{ gap: Array.isArray(gutter) ? gutter[0] : gutter, ...style }}>{children}</div>
);
export const Col = ({ children, span, xs, lg, className, style, ...rest }: AnyRecord) => (
  <div
    {...rest}
    className={cx('ui-col', className)}
    style={{
      '--ui-col-span': spanPercent(xs ?? span),
      '--ui-col-lg-span': spanPercent(lg),
      ...style,
    } as React.CSSProperties}
  >
    {children}
  </div>
);

type MenuComponentProps = BaseProps & MenuProps & {
  selectedKeys?: React.Key[];
  mode?: string;
  inlineCollapsed?: boolean;
};

export const Menu: React.FC<MenuComponentProps> = ({ items = [], onClick, className, selectedKeys = [] }) => (
  <div className={cx('ant-menu', 'ui-menu', className)}>
    {items.map((item: MenuItem) => <button key={String(item.key)} type="button" className={cx('ant-menu-item', 'ui-menu-item', selectedKeys.includes(item.key || '') && 'ui-menu-item-selected')} onClick={() => onClick?.({ key: String(item.key) })}>{item.icon}<span className="ant-menu-title-content">{item.label}</span></button>)}
  </div>
);

export const Drawer = ({ open, visible, title, children, onClose, width = 420, placement = 'right', className, ...rest }: AnyRecord) => (
  <DialogPrimitive.Root open={Boolean(open ?? visible)} onOpenChange={(nextOpen) => !nextOpen && onClose?.()}>
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="ui-modal-mask" />
      <DialogPrimitive.Content {...rest} className={cx('ui-drawer-content', `ui-drawer-${placement}`, className)} style={{ width, ...rest.style }}>
        <div className="ui-modal-header">
          <DialogPrimitive.Title className="ui-modal-title">{title}</DialogPrimitive.Title>
          <DialogPrimitive.Close asChild><button type="button" className="ui-modal-close">×</button></DialogPrimitive.Close>
        </div>
        <div className="ui-modal-body">{children}</div>
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  </DialogPrimitive.Root>
);

type UploadComponentProps = BaseProps & {
  beforeUpload?: (file: File, fileList?: File[]) => boolean | symbol | Promise<boolean | symbol | void> | void;
  onChange?: (info: { file: File; fileList: File[] }) => void;
  disabled?: boolean;
  multiple?: boolean;
  showUploadList?: boolean;
};

type UploadComponentType = React.FC<UploadComponentProps> & {
  Dragger: React.FC<UploadComponentProps>;
  LIST_IGNORE: symbol;
};

const UploadComponent: UploadComponentType = ({ children, beforeUpload, onChange, disabled, multiple, className }) => {
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const processFiles = async (files: File[]) => {
    for (const file of files) {
      const result = await beforeUpload?.(file, files);
      if (result === UploadComponent.LIST_IGNORE) continue;
      onChange?.({ file, fileList: files });
    }
  };
  return (
    <span
      className={className}
      onDragOver={(event) => {
        if (disabled) return;
        event.preventDefault();
      }}
      onDrop={async (event) => {
        if (disabled) return;
        event.preventDefault();
        await processFiles(Array.from(event.dataTransfer.files || []));
      }}
    >
      <input
        ref={inputRef}
        type="file"
        multiple={multiple}
        style={{ display: 'none' }}
        disabled={disabled}
        onChange={async (event) => {
          const files = Array.from(event.target.files || []);
          await processFiles(files);
          if (inputRef.current) inputRef.current.value = '';
        }}
      />
      {React.isValidElement(children)
        ? React.cloneElement(children as React.ReactElement<any>, { onClick: () => inputRef.current?.click() })
        : <Button onClick={() => inputRef.current?.click()}>{children}</Button>}
    </span>
  );
};
UploadComponent.Dragger = ({ children, className, ...props }) => (
  <UploadComponent {...props} className={cx('ui-upload-dragger', className)}>
    <div className="ui-upload-dragger-inner">{children}</div>
  </UploadComponent>
);
UploadComponent.LIST_IGNORE = Symbol('LIST_IGNORE');
export const Upload = UploadComponent;

export const Image = ({ src, alt, className, classNames, preview, ...rest }: AnyRecord) => {
  const [innerPreviewOpen, setInnerPreviewOpen] = React.useState(false);
  const previewConfig = preview && typeof preview === 'object' ? preview : {};
  const previewEnabled = preview !== false && preview !== undefined;
  const isPreviewControlled = previewConfig.visible !== undefined;
  const previewOpen = isPreviewControlled ? Boolean(previewConfig.visible) : innerPreviewOpen;
  const setPreviewOpen = (nextOpen: boolean) => {
    const previousOpen = previewOpen;
    if (!isPreviewControlled) setInnerPreviewOpen(nextOpen);
    previewConfig.onVisibleChange?.(nextOpen, previousOpen);
  };
  const previewSrc = previewConfig.src ?? src;
  const previewMask = previewConfig.mask;

  return (
    <>
      <span
        className={cx('ant-image', 'ui-image-root', previewEnabled && 'ui-image-preview-enabled', classNames?.root)}
        onClick={() => previewEnabled && setPreviewOpen(true)}
      >
        <img {...rest} src={src} alt={alt || ''} className={cx('ant-image-img', className, classNames?.image)} />
        {previewEnabled && previewMask !== false && (
          <span className="ant-image-mask ui-image-mask">
            {previewMask}
          </span>
        )}
      </span>
      {previewEnabled && (
        <DialogPrimitive.Root open={previewOpen} onOpenChange={setPreviewOpen}>
          <DialogPrimitive.Portal>
            <DialogPrimitive.Overlay className="ant-image-preview-mask ui-image-preview-mask" />
            <DialogPrimitive.Content className="ant-image-preview-wrap ui-image-preview-wrap">
              <DialogPrimitive.Title className="ui-sr-only">{alt || 'Image preview'}</DialogPrimitive.Title>
              <DialogPrimitive.Close asChild>
                <button type="button" className="ant-image-preview-close ui-image-preview-close">×</button>
              </DialogPrimitive.Close>
              <img src={previewSrc} alt={alt || ''} className="ant-image-preview-img ui-image-preview-img" />
            </DialogPrimitive.Content>
          </DialogPrimitive.Portal>
        </DialogPrimitive.Root>
      )}
    </>
  );
};

const toastRoot = () => {
  let root = document.querySelector('.ui-toast-root') as HTMLDivElement | null;
  if (!root) {
    root = document.createElement('div');
    root.className = 'ui-toast-root';
    document.body.appendChild(root);
  }
  return root;
};

const toastDestroyers = new Map<string, () => void>();

const showToast = (type: string, content: React.ReactNode, duration = 3, key?: string, onClose?: () => void) => {
  if (key) message.destroy(key);
  const container = document.createElement('div');
  container.className = cx('ui-toast', `ui-toast-${type}`);
  if (key) container.dataset.key = key;
  const root = createRoot(container);
  root.render(<>{content}</>);
  toastRoot().appendChild(container);
  const close = () => {
    if (key) toastDestroyers.delete(key);
    root.unmount();
    container.remove();
    onClose?.();
  };
  if (key) toastDestroyers.set(key, close);
  if (duration !== 0) window.setTimeout(close, duration * 1000);
  return close;
};

const normalizeMessageArgs = (type: string, args: any[]) => {
  const [first, duration, onClose] = args;
  if (first && typeof first === 'object' && !React.isValidElement(first) && ('content' in first || 'key' in first || 'duration' in first || 'type' in first)) {
    return {
      type: first.type || type,
      content: first.content,
      duration: first.duration,
      key: first.key,
      onClose: first.onClose,
    };
  }
  return { type, content: first, duration, onClose };
};

const messageTypeOpen = (type: string, ...args: any[]) => {
  const config = normalizeMessageArgs(type, args);
  return showToast(config.type, config.content, config.duration ?? (type === 'loading' ? 0 : 3), config.key, config.onClose);
};

export const message: any = {
  success: (...args: any[]) => messageTypeOpen('success', ...args),
  error: (...args: any[]) => messageTypeOpen('error', ...args),
  warning: (...args: any[]) => messageTypeOpen('warning', ...args),
  info: (...args: any[]) => messageTypeOpen('info', ...args),
  open: (config: AnyRecord) => messageTypeOpen(config.type || 'info', config),
  loading: (...args: any[]) => messageTypeOpen('loading', ...args),
  destroy: (key?: string) => {
    if (key) {
      toastDestroyers.get(key)?.();
      return;
    }
    Array.from(toastDestroyers.values()).forEach((destroy) => destroy());
    document.querySelectorAll('.ui-toast').forEach((node) => node.remove());
  },
};

export const notification: any = {
  info: ({ message: title, description, btn, duration }: AnyRecord) => showToast('info', <div><strong>{title}</strong><div>{description}</div>{btn}</div>, duration ?? 5),
  destroy: () => message.destroy(),
};

export const App: any = ({ children }: AnyRecord) => <>{children}</>;
App.useApp = () => ({ message, notification, modal: Modal });

export const ConfigProvider = ({ children }: AnyRecord) => <>{children}</>;

export const theme: any = {
  defaultAlgorithm: {},
  darkAlgorithm: {},
  useToken: () => ({
    token: {
      colorPrimary: 'var(--ant-color-primary)',
      colorText: 'var(--color-text-primary)',
      colorTextSecondary: 'var(--color-text-secondary)',
      colorBorder: 'var(--color-border)',
      colorBgContainer: 'var(--color-bg-container)',
    },
  }),
};

export default {
  Alert,
  App,
  AutoComplete,
  Button,
  Card,
  Checkbox,
  Col,
  Collapse,
  ConfigProvider,
  DatePicker,
  Descriptions,
  Divider,
  Drawer,
  Dropdown,
  Empty,
  Form,
  Input,
  InputNumber,
  List,
  Menu,
  Modal,
  Pagination,
  Popconfirm,
  Progress,
  Radio,
  Row,
  Segmented,
  Select,
  Space,
  Spin,
  Switch,
  Table,
  Tabs,
  Tag,
  Tooltip,
  Typography,
  Upload,
  message,
  notification,
  theme,
};
