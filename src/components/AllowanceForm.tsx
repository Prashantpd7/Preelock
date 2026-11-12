import { useState } from 'react';
import { useAppStore } from '../state/store';
import { Dropdown } from './Dropdown';
import { cleanDomain } from '../services/domains';
import { validateAllowanceForm } from '../services/allowances';
import type { Allowance } from '../types';

/** Create or edit a daily/weekly time allowance for one site or app. */
export function AllowanceForm({
  editing = null,
  onSaved,
  onCancel,
  className = 'panel form-panel simple-session',
}: {
  editing?: Allowance | null;
  onSaved: () => void;
  onCancel: () => void;
  className?: string;
}) {
  const { data, execute, busy } = useAppStore();
  const allowances = data.allowances ?? [];

  const [kind, setKind] = useState<'site' | 'app'>(editing?.kind ?? 'site');
  const [title, setTitle] = useState(editing?.title ?? '');
  const [target, setTarget] = useState(editing?.target ?? '');
  const [budget, setBudget] = useState(editing?.budget_minutes ?? 30);
  const [period, setPeriod] = useState<'day' | 'week'>(
    editing?.period ?? 'day',
  );
  const [mode, setMode] = useState<'rolling' | 'fixed'>(
    editing?.mode ?? 'rolling',
  );
  const [enabled, setEnabled] = useState(editing?.enabled ?? true);
  const [problem, setProblem] = useState('');

  async function submitAllowance(e: React.FormEvent) {
    e.preventDefault();
    setProblem('');
    // Site targets go through the same cleaner as the wizard's website
    // rules (scheme stripped, host lowercased); the backend still
    // validates, and rejects paths and search: keywords.
    const normalizedTarget =
      kind === 'site' ? (cleanDomain(target) ?? target.trim()) : target.trim();
    const validationProblem = validateAllowanceForm({
      title,
      kind,
      target: normalizedTarget,
      budgetMinutes: budget,
      period,
    });
    if (validationProblem) {
      setProblem(validationProblem);
      return;
    }
    if (editing) {
      const existing = allowances.find((a) => a.id === editing.id);
      if (!existing) {
        setProblem('That allowance no longer exists.');
        return;
      }
      if (
        await execute({
          type: 'update_allowance',
          allowance: {
            ...existing,
            title: title.trim(),
            kind,
            target: normalizedTarget,
            budget_minutes: budget,
            period,
            mode,
            enabled,
          },
        })
      ) {
        onSaved();
      }
      return;
    }
    if (
      await execute({
        type: 'create_allowance',
        title: title.trim(),
        kind,
        target: normalizedTarget,
        budget_minutes: budget,
        period,
        mode,
        enabled,
      })
    ) {
      onSaved();
    }
  }

  return (
    <form className={className} onSubmit={submitAllowance}>
      <h2>{editing ? 'Edit allowance' : 'New allowance'}</h2>
      <label className="grow">
        Allowance name
        <input
          required
          maxLength={160}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="e.g. YouTube"
        />
      </label>
      <label>
        Allow for
        <Dropdown
          value={kind}
          onChange={(value) => {
            setKind(value as 'site' | 'app');
            setProblem('');
          }}
          options={[
            { value: 'site', label: 'A site' },
            { value: 'app', label: 'An app' },
          ]}
        />
      </label>
      <label className="grow">
        {kind === 'site' ? 'Site' : 'App id'}
        <input
          required
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          placeholder={
            kind === 'site'
              ? 'e.g. youtube.com or *.example.com'
              : 'e.g. com.example.game'
          }
        />
      </label>
      {kind === 'site' && (
        <p className="small muted">Domain only; no paths or searches.</p>
      )}
      <div className="simple-grid allowance-grid">
        <label>
          Allow in minutes
          <input
            type="number"
            required
            min={1}
            max={period === 'day' ? 1440 : 10080}
            value={budget}
            onChange={(e) => setBudget(Number(e.target.value))}
          />
        </label>
        <label>
          Period
          <Dropdown
            value={period}
            onChange={(value) => setPeriod(value as 'day' | 'week')}
            options={[
              { value: 'day', label: 'Daily' },
              { value: 'week', label: 'Weekly' },
            ]}
          />
        </label>
        <label>
          Reset
          <Dropdown
            value={mode}
            onChange={(value) => setMode(value as 'rolling' | 'fixed')}
            options={[
              {
                value: 'rolling',
                label: 'Rolling (last 24h / 7 days)',
              },
              {
                value: 'fixed',
                label: 'Fixed (calendar day / Monday week)',
              },
            ]}
          />
        </label>
      </div>
      <label className="check-label">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => setEnabled(e.target.checked)}
        />
        Enabled
      </label>
      {problem && (
        <p className="error small" role="alert">
          {problem}
        </p>
      )}
      <div className="row controls" style={{ justifyContent: 'flex-end' }}>
        <button type="button" className="secondary" onClick={onCancel}>
          Cancel
        </button>
        <button disabled={busy}>
          {editing ? 'Save allowance' : 'Create allowance'}
        </button>
      </div>
    </form>
  );
}
