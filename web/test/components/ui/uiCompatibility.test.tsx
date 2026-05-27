import React, { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AutoComplete, Button, DatePicker, Form, Input, message, Modal, Table, Tabs, Upload } from '../../../components/ui';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type RenderedRoot = {
  container: HTMLDivElement;
  root: Root;
};

const renderedRoots: RenderedRoot[] = [];

const flush = async () => {
  await act(async () => {
    await Promise.resolve();
  });
};

const render = (element: React.ReactElement) => {
  const container = document.createElement('div');
  document.body.appendChild(container);
  let root: Root;
  act(() => {
    root = createRoot(container);
    root.render(element);
  });
  const rendered = { container, root: root! };
  renderedRoots.push(rendered);
  return rendered;
};

const changeInputValue = (input: HTMLInputElement, value: string) => {
  act(() => {
    const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    valueSetter?.call(input, value);
    input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: value }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
};

const clickElement = (element: Element) => {
  act(() => {
    if (element instanceof HTMLButtonElement && element.disabled) {
      element.click();
      return;
    }
    element.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, cancelable: true, button: 0 }));
    element.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, button: 0 }));
    element.dispatchEvent(new MouseEvent('pointerup', { bubbles: true, cancelable: true, button: 0 }));
    element.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, button: 0 }));
    element.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 }));
  });
};

afterEach(() => {
  Modal.destroyAll();
  message.destroy();
  act(() => {
    while (renderedRoots.length > 0) {
      const rendered = renderedRoots.pop();
      rendered?.root.unmount();
      rendered?.container.remove();
    }
  });
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('local UI compatibility layer', () => {
  it('preserves Form.List prefixes, initialValue, validators, and full onChange arguments', async () => {
    let submittedValues: any;
    let multiArgEvent: [string, boolean] | undefined;
    const validator = vi.fn(async () => undefined);

    const MultiArgControl = ({ value, onChange }: { value?: string; onChange?: (value: string, isValid: boolean) => void }) => (
      <button type="button" data-testid="multi-arg" onClick={() => onChange?.('next-json', true)}>
        {value || 'empty'}
      </button>
    );

    const { container } = render(
      <Form
        initialValues={{ args: [''] }}
        onFinish={(values) => {
          submittedValues = values;
        }}
      >
        <Form.List name="args">
          {(fields) => (
            <>
              {fields.map((field) => (
                <Form.Item {...field} key={field.key} noStyle>
                  <Input aria-label={`arg-${field.name}`} />
                </Form.Item>
              ))}
            </>
          )}
        </Form.List>
        <Form.Item name="sdkType" initialValue="openai" noStyle>
          <Input aria-label="sdk" />
        </Form.Item>
        <Form.Item
          name="extraOptions"
          noStyle
          getValueFromEvent={(value: string, isValid: boolean) => {
            multiArgEvent = [value, isValid];
            return value;
          }}
        >
          <MultiArgControl />
        </Form.Item>
        <Form.Item name="model" noStyle rules={[{ validator }]}>
          <Input aria-label="model" />
        </Form.Item>
        <Button htmlType="submit">Submit</Button>
      </Form>,
    );

    await flush();
    changeInputValue(container.querySelector('input[aria-label="arg-0"]') as HTMLInputElement, 'stdio');
    changeInputValue(container.querySelector('input[aria-label="model"]') as HTMLInputElement, 'gpt-5');
    clickElement(container.querySelector('[data-testid="multi-arg"]') as Element);
    clickElement(container.querySelector('button[type="submit"]') as Element);
    await flush();

    expect(validator).toHaveBeenCalledWith(expect.any(Object), 'gpt-5');
    expect(multiArgEvent).toEqual(['next-json', true]);
    expect(submittedValues).toMatchObject({
      args: ['stdio'],
      sdkType: 'openai',
      extraOptions: 'next-json',
      model: 'gpt-5',
    });
  });

  it('blocks submit when custom Form validators reject', async () => {
    const onFinish = vi.fn();

    const { container } = render(
      <Form onFinish={onFinish}>
        <Form.Item name="toml" noStyle rules={[{ validator: async () => Promise.reject(new Error('invalid toml')) }]}>
          <Input aria-label="toml" />
        </Form.Item>
        <Button htmlType="submit">Submit</Button>
      </Form>,
    );

    await flush();
    changeInputValue(container.querySelector('input[aria-label="toml"]') as HTMLInputElement, '[broken');
    clickElement(container.querySelector('button[type="submit"]') as Element);
    await flush();

    expect(onFinish).not.toHaveBeenCalled();
  });

  it('returns date-like values from RangePicker changes', () => {
    let capturedDates: any[] | null | undefined;
    let capturedStrings: [string, string] | undefined;

    const { container } = render(
      <DatePicker.RangePicker
        value={null}
        onChange={(dates, dateStrings) => {
          capturedDates = dates as any[] | null | undefined;
          capturedStrings = dateStrings;
        }}
      />,
    );

    const [startInput] = Array.from(container.querySelectorAll('input'));
    changeInputValue(startInput as HTMLInputElement, '2026-05-01');

    expect(capturedStrings).toEqual(['2026-05-01', '']);
    expect(capturedDates?.[0]).toEqual(expect.objectContaining({ toDate: expect.any(Function) }));
    expect(capturedDates?.[0].toDate()).toBeInstanceOf(Date);
  });

  it('keeps AutoComplete freely editable for custom values', () => {
    const onChange = vi.fn();

    const { container } = render(
      <AutoComplete options={[{ value: 'known-model', label: 'known-model' }]} onChange={onChange} placeholder="model" />,
    );

    const input = container.querySelector('input') as HTMLInputElement;
    changeInputValue(input, 'custom-model');

    expect(input.value).toBe('custom-model');
    expect(onChange).toHaveBeenLastCalledWith('custom-model', undefined);
  });

  it('passes disabled and loading props to Modal default buttons', () => {
    const onOk = vi.fn();

    render(
      <Modal open title="Config" okButtonProps={{ disabled: true }} confirmLoading onOk={onOk}>
        Body
      </Modal>,
    );

    const okButton = Array.from(document.body.querySelectorAll('button')).find((button) => button.textContent?.includes('OK')) as HTMLButtonElement;
    expect(okButton.disabled).toBe(true);
    clickElement(okButton);
    expect(onOk).not.toHaveBeenCalled();
  });

  it('calls beforeUpload for every selected file when multiple is enabled', async () => {
    const beforeUpload = vi.fn(() => false);
    const onChange = vi.fn();

    const { container } = render(
      <Upload multiple beforeUpload={beforeUpload} onChange={onChange}>
        <Button>Select</Button>
      </Upload>,
    );
    const input = container.querySelector('input[type="file"]') as HTMLInputElement;
    const files = [new File(['a'], 'a.png'), new File(['b'], 'b.png')];
    Object.defineProperty(input, 'files', { value: files, configurable: true });

    await act(async () => {
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });

    expect(beforeUpload).toHaveBeenCalledTimes(2);
    expect(beforeUpload.mock.calls.map((call) => ((call as unknown as [File])[0]).name)).toEqual(['a.png', 'b.png']);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('honors rowSelection.getCheckboxProps for disabled table rows and header selection', () => {
    const onChange = vi.fn();

    const { container } = render(
      <Table
        rowKey="id"
        columns={[{ title: 'Name', dataIndex: 'name' }]}
        dataSource={[
          { id: 'existing', name: 'Existing' },
          { id: 'new', name: 'New' },
        ]}
        rowSelection={{
          selectedRowKeys: [],
          onChange,
          getCheckboxProps: (record: { id: string }) => ({ disabled: record.id === 'existing' }),
        }}
      />,
    );

    const checkboxes = Array.from(container.querySelectorAll<HTMLButtonElement>('.ui-checkbox'));
    expect(checkboxes[1].disabled).toBe(true);
    clickElement(checkboxes[0]);

    expect(onChange).toHaveBeenCalledWith(['new'], [{ id: 'new', name: 'New' }]);
  });

  it('uses defaultActiveKey as an uncontrolled Tabs default and renders ReactNode extra content', () => {
    const onChange = vi.fn();

    const { container } = render(
      <Tabs
        defaultActiveKey="one"
        onChange={onChange}
        tabBarExtraContent={<button type="button">Reset</button>}
        items={[
          { key: 'one', label: 'One', children: <div>One panel</div> },
          { key: 'two', label: 'Two', children: <div>Two panel</div> },
        ]}
      />,
    );

    expect(container.textContent).toContain('Reset');
    const oneTab = Array.from(container.querySelectorAll('button')).find((button) => button.textContent?.includes('One')) as HTMLButtonElement;
    const twoTab = Array.from(container.querySelectorAll('button')).find((button) => button.textContent?.includes('Two')) as HTMLButtonElement;
    expect(oneTab.dataset.state).toBe('active');

    clickElement(twoTab);

    expect(onChange).toHaveBeenCalledWith('two');
    expect(twoTab.dataset.state).toBe('active');
  });

  it('supports keyed message config overloads', async () => {
    act(() => {
      message.loading({ key: 'export', content: 'Exporting' });
    });
    await flush();
    expect(document.body.textContent).toContain('Exporting');

    act(() => {
      message.success({ key: 'export', content: 'Exported' });
    });
    await flush();

    expect(document.body.textContent).toContain('Exported');
    expect(document.body.textContent).not.toContain('Exporting');
  });
});
