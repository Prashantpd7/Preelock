import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it } from 'vitest';
import { NumberInput } from '../components/NumberInput';

function NumberInputHarness() {
  const [value, setValue] = useState(0);
  return (
    <NumberInput
      aria-label="Minutes"
      value={value}
      onValueChange={(raw) => {
        if (raw !== '') setValue(Number(raw));
      }}
    />
  );
}

it('allows deleting the full value and entering a number without leading zeros', async () => {
  const user = userEvent.setup();
  render(<NumberInputHarness />);
  const input = screen.getByRole('spinbutton', { name: 'Minutes' });

  await user.clear(input);
  await user.type(input, '120');
  await user.keyboard('{Backspace}{Backspace}{Backspace}');
  expect(input).toHaveValue(null);

  await user.type(input, '0120');
  expect(input).toHaveValue(120);
});
