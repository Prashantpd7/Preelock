import { useState } from 'react';
import { Plus } from 'lucide-react';
import { PageHeading } from '../components/ui';
import { TaskList } from '../components/TaskList';
import { useAppStore } from '../state/store';
import { desktop } from '../services/api';
export function Tasks() {
  const { data, execute, busy } = useAppStore();
  const [title, setTitle] = useState('');
  return (
    <>
      <PageHeading
        eyebrow="TURN INTENTION INTO PROGRESS"
        title="Tasks"
        description="A clear list for a clearer mind. Keep it achievable."
      />
      <form
        className="panel inline-form"
        onSubmit={async (e) => {
          e.preventDefault();
          if (await execute({ type: 'create_task', title })) setTitle('');
        }}
      >
        <input
          className="grow"
          aria-label="New task"
          placeholder="What needs your attention?"
          value={title}
          required
          maxLength={160}
          onChange={(e) => setTitle(e.target.value)}
        />
        <button disabled={busy || !desktop}>
          <Plus size={16} />
          Add task
        </button>
      </form>
      <div className="section-heading">
        <h2>Your tasks</h2>
        <span className="muted small">
          {data.tasks.filter((t) => t.completed).length} of {data.tasks.length}{' '}
          complete
        </span>
      </div>
      <div className="panel">
        <TaskList tasks={data.tasks} />
      </div>
    </>
  );
}
