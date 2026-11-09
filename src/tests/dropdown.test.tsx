import { useState } from 'react';
import { expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Dropdown, type DropdownOption } from '../components/Dropdown';

const OPTIONS: DropdownOption[] = [
  { value: 'a', label: 'First option' },
  { value: 'b', label: 'Second option' },
  { value: 'c', label: 'Third option' },
];

function Controlled({ disabled }: { disabled?: boolean }) {
  const [value, setValue] = useState('a');
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
      }}
    >
      <label>
        Pick one
        <Dropdown
          value={value}
          options={OPTIONS}
          onChange={setValue}
          disabled={disabled}
        />
      </label>
      <button type="submit">Save</button>
    </form>
  );
}

it('opens the app-rendered panel and shows every option', async () => {
  const user = userEvent.setup();
  render(<Controlled />);
  expect(screen.queryByRole('listbox')).toBeNull();
  await user.click(screen.getByRole('button', { name: 'Pick one' }));
  expect(screen.getByRole('listbox')).toBeInTheDocument();
  for (const option of OPTIONS) {
    expect(
      screen.getByRole('option', { name: option.label }),
    ).toBeInTheDocument();
  }
});

it('selecting an option updates the value, closes, and never submits the form', async () => {
  const user = userEvent.setup();
  const onSubmit = vi.fn((e: React.FormEvent) => e.preventDefault());
  render(
    <form onSubmit={onSubmit}>
      <Dropdown value="a" options={OPTIONS} onChange={() => {}} />
    </form>,
  );
  // Standalone (no wrapping label): the trigger shows the selected label.
  await user.click(screen.getByRole('button', { name: 'First option' }));
  await user.click(screen.getByRole('option', { name: 'Third option' }));
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(onSubmit).not.toHaveBeenCalled();
});

it('selection through the controlled wrapper updates the trigger text', async () => {
  const user = userEvent.setup();
  render(<Controlled />);
  const trigger = screen.getByRole('button', { name: 'Pick one' });
  await user.click(trigger);
  await user.click(screen.getByRole('option', { name: 'Second option' }));
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(trigger.textContent).toBe('Second option');
});

it('Escape closes the panel and focus returns to the trigger', async () => {
  const user = userEvent.setup();
  render(<Controlled />);
  const trigger = screen.getByRole('button', { name: 'Pick one' });
  await user.click(trigger);
  expect(screen.getByRole('listbox')).toBeInTheDocument();
  await user.keyboard('{Escape}');
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(trigger).toHaveFocus();
});

it('clicking outside the panel (the backdrop) closes it', async () => {
  const user = userEvent.setup();
  const { container } = render(<Controlled />);
  await user.click(screen.getByRole('button', { name: 'Pick one' }));
  expect(screen.getByRole('listbox')).toBeInTheDocument();
  const backdrop = container.querySelector('.dropdown-backdrop');
  expect(backdrop).not.toBeNull();
  await user.click(backdrop!);
  expect(screen.queryByRole('listbox')).toBeNull();
});

it('keyboard navigation moves the highlight and Enter selects it', async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Dropdown value="a" options={OPTIONS} onChange={onChange} />);
  const trigger = screen.getByRole('button', { name: 'First option' });
  trigger.focus();
  await user.keyboard('{ArrowDown}');
  expect(screen.getByRole('listbox')).toBeInTheDocument();
  await user.keyboard('{ArrowDown}');
  const activeId = trigger.getAttribute('aria-activedescendant');
  expect(activeId).not.toBeNull();
  const active = document.getElementById(activeId!);
  expect(active?.textContent).toBe('Second option');
  await user.keyboard('{Enter}');
  expect(onChange).toHaveBeenCalledWith('b');
  expect(screen.queryByRole('listbox')).toBeNull();
});

it('Tab closes the panel and moves focus on', async () => {
  const user = userEvent.setup();
  render(<Controlled />);
  await user.click(screen.getByRole('button', { name: 'Pick one' }));
  expect(screen.getByRole('listbox')).toBeInTheDocument();
  await user.tab();
  expect(screen.queryByRole('listbox')).toBeNull();
  expect(screen.getByRole('button', { name: 'Save' })).toHaveFocus();
});

it('a disabled dropdown does not open', async () => {
  const user = userEvent.setup();
  render(<Controlled disabled />);
  await user.click(screen.getByRole('button', { name: 'Pick one' }));
  expect(screen.queryByRole('listbox')).toBeNull();
});
