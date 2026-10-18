import { useState } from 'react';
import { Check, Pencil, X } from 'lucide-react';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
import type { Task } from '../types';
import { Empty } from './ui';
function TaskRow({ task }: { task: Task }) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(task.title);
  const execute = useAppStore((s) => s.execute);
  const busy = useAppStore((s) => s.busy);
  return (
    <li className="task-row">
      <input
        type="checkbox"
        checked={task.completed}
        aria-label={`Complete ${task.title}`}
        disabled={busy || !desktop}
        onChange={() =>
          void execute({
            type: 'update_task',
            ...task,
            completed: !task.completed,
          })
        }
      />
      {editing ? (
        <form
          className="inline-form grow"
          onSubmit={async (e) => {
            e.preventDefault();
            if (await execute({ type: 'update_task', ...task, title }))
              setEditing(false);
          }}
        >
          <input
            autoFocus
            aria-label="Task title"
            value={title}
            maxLength={160}
            required
            onChange={(e) => setTitle(e.target.value)}
          />
          <button aria-label="Save task" disabled={busy}>
            <Check size={16} />
          </button>
          <button
            type="button"
            className="secondary"
            aria-label="Cancel editing"
            onClick={() => setEditing(false)}
          >
            <X size={16} />
          </button>
        </form>
      ) : (
        <>
          <span className={`grow ${task.completed ? 'done' : ''}`}>
            {task.title}
          </span>
          <button
            className="icon-button"
            aria-label={`Edit ${task.title}`}
            onClick={() => {
              setTitle(task.title);
              setEditing(true);
            }}
          >
            <Pencil size={15} />
          </button>
        </>
      )}
    </li>
  );
}
export function TaskList({ tasks }: { tasks: Task[] }) {
  return tasks.length ? (
    <ul className="task-list">
      {tasks.map((task) => (
        <TaskRow key={task.id} task={task} />
      ))}
    </ul>
  ) : (
    <Empty>No tasks yet. Give your next focus session a purpose.</Empty>
  );
}
